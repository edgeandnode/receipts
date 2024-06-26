use std::fmt;

use rand::RngCore;
use secp256k1::SecretKey;

use crate::prelude::*;

// Keep track of the offsets to index the data in an array.
// I'm really happy with how this turned out to make book-keeping easier.
// A macro might make this better though.
const ALLOCATION_ID_RANGE: Range = next_range::<Address>(0..0);
const FEE_RANGE: Range = next_range::<U256>(ALLOCATION_ID_RANGE);
const RECEIPT_ID_RANGE: Range = next_range::<ReceiptId>(FEE_RANGE);
const SIGNATURE_RANGE: Range = next_range::<Signature>(RECEIPT_ID_RANGE);
const UNLOCKED_FEE_RANGE: Range = next_range::<U256>(SIGNATURE_RANGE);
pub const BORROWED_RECEIPT_LEN: usize = UNLOCKED_FEE_RANGE.end;

/// A per-allocation collection that can borrow or generate receipts.
#[derive(Debug, PartialEq, Eq)]
pub struct ReceiptPool {
    pub allocation: Address,
    /// Receipts that can be folded. These contain an unbroken chain
    /// of agreed upon history between the Indexer and Gateway.
    receipt_cache: Vec<PooledReceipt>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum QueryStatus {
    Success,
    Failure,
    Unknown,
}

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct PooledReceipt {
    pub unlocked_fee: U256,
    pub receipt_id: ReceiptId,
}

#[derive(Eq, PartialEq, Debug)]
pub enum BorrowFail {
    NoAllocation,
    InvalidRecoveryId,
}

impl std::error::Error for BorrowFail {}

impl fmt::Display for BorrowFail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAllocation => write!(f, "No allocation"),
            Self::InvalidRecoveryId => SignError::InvalidRecoveryId.fmt(f),
        }
    }
}

impl From<SignError> for BorrowFail {
    fn from(err: SignError) -> Self {
        match err {
            SignError::InvalidRecoveryId => Self::InvalidRecoveryId,
        }
    }
}

impl ReceiptPool {
    pub fn new(allocation: Address) -> Self {
        Self {
            allocation,
            receipt_cache: Default::default(),
        }
    }

    /// This is only a minimum bound, and doesn't count
    /// outstanding/forgotten receipts which may have account for a
    /// significant portion of unlocked fees
    #[cfg(test)]
    pub fn known_unlocked_fees(&self) -> U256 {
        let mut result = U256::zero();
        for fee in self.receipt_cache.iter().map(|r| r.unlocked_fee) {
            result += fee;
        }
        result
    }

    pub fn commit(&mut self, signer: &SecretKey, locked_fee: U256) -> Result<Vec<u8>, BorrowFail> {
        let receipt = if self.receipt_cache.is_empty() {
            let mut receipt_id = ReceiptId::default();
            rng().fill_bytes(&mut receipt_id);
            PooledReceipt {
                receipt_id,
                unlocked_fee: U256::zero(),
            }
        } else {
            let receipts = &mut self.receipt_cache;
            let index = rng().gen_range(0..receipts.len());
            receipts.swap_remove(index)
        };

        // Technically we don't need the mutable borrow from here on out.
        // If we ever need to unlock more concurency when these are locked
        // it would be possible to split out the remainder of this method.

        // Write the data in the official receipt that gets sent over the wire.
        // This is: [allocation_id, fee, receipt_id, signature]
        let mut commitment = Vec::with_capacity(BORROWED_RECEIPT_LEN);
        let fee = receipt.unlocked_fee + locked_fee;
        commitment.extend_from_slice(&self.allocation);
        commitment.extend_from_slice(&to_be_bytes(fee));
        commitment.extend_from_slice(&receipt.receipt_id);

        // Engineering in any kind of replay protection like as afforded by EIP-712 is
        // unnecessary, because the signer key needs to be unique per app. It is a straightforward
        // extension from there to also say that the signer key should be globally unique and
        // not sign any messages that are not for the app. Since there are no other structs
        // to sign, there are no possible collisions.
        //
        // The part of the message that needs to be signed in the fee and receipt id only.
        let signature = sign(
            &commitment[ALLOCATION_ID_RANGE.start..RECEIPT_ID_RANGE.end],
            signer,
        )?;
        commitment.extend_from_slice(&signature);

        // Extend with the unlocked fee, which is necessary to return collateral
        // in the case of failure.
        commitment.extend_from_slice(&to_be_bytes(receipt.unlocked_fee));

        debug_assert_eq!(BORROWED_RECEIPT_LEN, commitment.len());

        Ok(commitment)
    }

    pub fn release(&mut self, bytes: &[u8], status: QueryStatus) {
        assert_eq!(bytes.len(), BORROWED_RECEIPT_LEN);

        let unlocked_fee = if status == QueryStatus::Success {
            U256::from_big_endian(&bytes[FEE_RANGE])
        } else {
            U256::from_big_endian(&bytes[UNLOCKED_FEE_RANGE])
        };

        let receipt = PooledReceipt {
            unlocked_fee,
            receipt_id: bytes[RECEIPT_ID_RANGE].try_into().unwrap(),
        };
        self.receipt_cache.push(receipt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::*;

    #[track_caller]
    fn assert_successful_borrow(pool: &mut ReceiptPool, fee: impl Into<U256>) -> Vec<u8> {
        pool.commit(&test_signer(), fee.into())
            .expect("Should be able to borrow")
    }

    // Simple happy-path case of paying for requests in a loop.
    #[test]
    pub fn can_pay_for_requests() {
        let mut pool = ReceiptPool::new(bytes(1));

        for i in 1..=10 {
            let borrow = assert_successful_borrow(&mut pool, i);
            pool.release(&borrow, QueryStatus::Success);
            // Verify that we have unlocked all the fees
            let unlocked: u32 = (0..=i).sum();
            assert_eq!(U256::from(unlocked), pool.known_unlocked_fees());
        }
    }

    // Tests modes for returning collateral
    #[test]
    fn collateral_return() {
        let mut pool = ReceiptPool::new(bytes(2));

        let borrow3 = assert_successful_borrow(&mut pool, 3);
        assert_eq!(pool.known_unlocked_fees(), 0.into());

        let borrow2 = assert_successful_borrow(&mut pool, 2);
        assert_eq!(pool.known_unlocked_fees(), 0.into());

        pool.release(&borrow3, QueryStatus::Failure);
        assert_eq!(pool.known_unlocked_fees(), 0.into());

        let borrow4 = assert_successful_borrow(&mut pool, 4);
        assert_eq!(pool.known_unlocked_fees(), 0.into());

        pool.release(&borrow2, QueryStatus::Success);
        assert_eq!(pool.known_unlocked_fees(), 2.into());

        pool.release(&borrow4, QueryStatus::Unknown);
        assert_eq!(pool.known_unlocked_fees(), 2.into());
    }
}
