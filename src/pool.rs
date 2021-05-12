use crate::prelude::*;
use lazy_static::lazy_static;
use secp256k1::{Message, Secp256k1, SecretKey, SignOnly};
use std::mem::size_of;

lazy_static! {
    static ref SIGNER: Secp256k1<SignOnly> = Secp256k1::signing_only();
}

type Range = std::ops::Range<usize>;

const fn next_range<T>(prev: Range) -> Range {
    prev.end..prev.end + size_of::<T>()
}

// Keep track of the offsets to index the data in an array.
// I'm really happy with how this turned out to make book-keeping easier.
// A macro might make this better though.
const VECTOR_TRANSFER_ID_RANGE: Range = next_range::<Bytes32>(0..0);
const FEE_RANGE: Range = next_range::<U256>(VECTOR_TRANSFER_ID_RANGE);
const RECEIPT_ID_RANGE: Range = next_range::<ReceiptID>(FEE_RANGE);
const SIGNATURE_RANGE: Range = next_range::<[u8; 65]>(RECEIPT_ID_RANGE);
const UNLOCKED_FEE_RANGE: Range = next_range::<U256>(SIGNATURE_RANGE);
pub const BORROWED_RECEIPT_LEN: usize = UNLOCKED_FEE_RANGE.end;

/// A collection of installed transfers that can borrow or generate receipts.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct ReceiptPool {
    transfers: Vec<Transfer>,
}

// The maximum receipt id allowed by the size limit.
// For now, the size limit is 50MiB for a particular Connext
// API when resolving using uncompressed JSON. We could
// calculate a hard limit here since every value has bounds.
// (eg: We know exactly how much the numbers 1-200000 in JSON are exactly,
// and how much the fee field would be if each was set to U256 MAX).
// But instead this is just a conservative estimate. That leaves room
// for Connext to change their APIs and introduce more overhead without
// creating a security hole.
//
// Even when there is a ZKP there will be a maximum receipt id, but
// it could be much higher.
const MAX_RECEIPT_ID: u32 = 200000;
const LOW_RECEIPT_CAPACITY: u32 = 170000;

/// A in-flight state for a transfer that has been initiated using Vector.
// This must never implement Clone
#[derive(Debug, PartialEq, Eq)]
struct Transfer {
    /// Collateral when the transfer was created
    initial_collateral: U256,
    /// Used to detect when collateral is running out
    low_collateral_threshold: U256,
    /// There is no need to sync the collateral to the db. If we crash, should
    /// either rotate out transfers or recover the transfer state from the Indexer.
    remaining_collateral: U256,
    /// The ZKP is most efficient when using receipts from a contiguous range
    /// as this allows the receipts to be constants rather than witnessed-in,
    /// and also have preset data sizes for amortized proving time.
    prev_receipt_id: ReceiptID,
    /// Receipts that can be folded. These contain an unbroken chain
    /// of agreed upon history between the Indexer and Gateway.
    receipt_cache: Vec<PooledReceipt>,
    /// Signer: Signs the receipts. Each transfer must have a globally unique ppk pair.
    /// If keys are shared across multiple Transfers it will allow the Indexer to
    /// double-collect the same receipt across multiple transfers
    signer: SecretKey,
    /// The address should be enough to uniquely identify a transfer,
    /// since the security model of the Consumer relies on having a unique
    /// address for each transfer. But, there's nothing preventing a Consumer
    /// from putting themselves at risk - which opens up some interesting
    /// greifing attacks. It just makes it simpler therefore to use the transfer
    /// id to key the receipt and lookup the address rather than lookup all transfers
    /// matching an address. Unlike the address which is specified by the Consumer,
    /// the transfer id has a unique source of truth - the Vector node.
    vector_transfer_id: Bytes32,
}

#[derive(PartialEq, Eq, Debug)]
pub struct ReceiptBorrow {
    /// The actual data that would unlock the fee.
    /// Because of JS interop this also includes some extra metadata
    pub commitment: Vec<u8>,
    /// This will be true exactly once per transfer when
    /// that transfer crosses over a threshold for low collateral.
    pub low_collateral_warning: bool,
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
    pub receipt_id: ReceiptID,
}

#[derive(Eq, PartialEq, Debug)]
pub enum BorrowFail {
    // If this error is encountered it means that a new transfer with
    // more collateral must be installed.
    InsufficientCollateral,
}

impl ReceiptPool {
    pub fn new() -> Self {
        Self {
            transfers: Vec::new(),
        }
    }

    /// This is only a minimum bound, and doesn't count
    /// outstanding/forgotten receipts which may have account for a
    /// significant portion of the unlocked fees
    #[cfg(test)]
    pub fn known_unlocked_fees(&self) -> U256 {
        let mut fees = U256::zero();
        for fee in self
            .transfers
            .iter()
            .flat_map(|a| &a.receipt_cache)
            .map(|r| r.unlocked_fee)
        {
            fees += fee;
        }
        fees
    }

    pub fn add_transfer(
        &mut self,
        signer: SecretKey,
        collateral: U256,
        vector_transfer_id: Bytes32,
    ) {
        // Defensively ensure we don't already have this transfer.
        // Note that the collateral may not match, but that would be ok.
        for transfer in self.transfers.iter() {
            if transfer.vector_transfer_id == vector_transfer_id {
                return;
            }
        }
        let transfer = Transfer {
            signer,
            initial_collateral: collateral,
            remaining_collateral: collateral,
            low_collateral_threshold: collateral / 4,
            receipt_cache: Vec::new(),
            prev_receipt_id: 0,
            vector_transfer_id,
        };
        self.transfers.push(transfer)
    }

    pub fn remove_transfer(&mut self, vector_transfer_id: &Bytes32) {
        if let Some(index) = self
            .transfers
            .iter()
            .position(|a| &a.vector_transfer_id == vector_transfer_id)
        {
            self.transfers.swap_remove(index);
        }
    }

    pub fn has_collateral_for(&self, locked_fee: U256) -> bool {
        self.transfers.iter().any(|a| {
            a.remaining_collateral >= locked_fee
                && (a.receipt_cache.len() > 0 || a.prev_receipt_id != MAX_RECEIPT_ID)
        })
    }

    pub fn recommended_collateral(&self) -> U256 {
        // Went through a couple of options here.
        // The most succinct answer to the question of
        // how much collateral we need is just "more than we are using".
        // So multiply the used amount by 2. Min/Max is dealt with outside
        // in the environment return 0 or U256::MAX is fine.
        //
        // One important criteria is that the collateral increases geometrically
        // when we rotate a transfer out early for low collateral. The low
        // collateral warning goes out at 3/4 of the collateral so doubling
        // at that moment gives us 1.5x. If the next transfer runs out of
        // collateral before the previous is removed then collateral grows
        // quicker, which is good.
        //
        // The one thing this doesn't do is to gracefully degrade. Instead,
        // it can drop off fast with no usage. I think that's actually ok.
        // This isn't TCP and for the expected lifetime of transfers a large
        // drop in volume should sample a large enough time to be considered
        // significant.
        let mut used = U256::zero();

        for transfer in self.transfers.iter() {
            used = used.saturating_add(transfer.initial_collateral - transfer.remaining_collateral);
        }

        used.saturating_mul(2.into())
    }

    // Uses the transfer with the least collateral that can sustain the fee.
    // This is to ensure low-latency rollover between transfers, and keeping number
    // of transfers installed at any given time to a minimum. To understand, consider
    // what would happen if we selected the transfer with the highest collateral -
    // transfers would run out of collateral at the same time.
    // Random transfer selection is not much better than the worst case.
    fn select_transfer(&mut self, locked_fee: U256) -> Option<&mut Transfer> {
        let mut selected_transfer = None;
        for transfer in self.transfers.iter_mut() {
            if transfer.remaining_collateral < locked_fee {
                continue;
            }

            match selected_transfer {
                None => selected_transfer = Some(transfer),
                Some(ref mut selected) => {
                    if selected.remaining_collateral > transfer.remaining_collateral {
                        *selected = transfer;
                    }
                }
            }
        }
        selected_transfer
    }

    fn transfer_by_id_mut(&mut self, vector_transfer_id: &Bytes32) -> Option<&mut Transfer> {
        self.transfers
            .iter_mut()
            .find(|a| &a.vector_transfer_id == vector_transfer_id)
    }

    pub fn commit(&mut self, locked_fee: U256) -> Result<ReceiptBorrow, BorrowFail> {
        let transfer = self
            .select_transfer(locked_fee)
            .ok_or(BorrowFail::InsufficientCollateral)?;
        let low_before = transfer.remaining_collateral < transfer.low_collateral_threshold;
        transfer.remaining_collateral -= locked_fee;
        let mut low_now = transfer.remaining_collateral < transfer.low_collateral_threshold;

        let receipt = if transfer.receipt_cache.len() == 0 {
            // This starts with the id of 1 because the transfer definition contract
            // was written in a way that makes the receipt id of 0 invalid.
            //
            // Technically running out of ids is not "insufficient collateral",
            // but it kind of is if you consider the collateral
            // to be inaccessible. Also the mitigation is the same -
            // rotating apps.
            match transfer.prev_receipt_id {
                MAX_RECEIPT_ID => {
                    // Do not produce a receipt if it would exceed MAX_RECEIPT_ID
                    // Put the unused collateral back into the transfer since it
                    // is not locked in a receipt.
                    transfer.remaining_collateral += locked_fee;
                    return Err(BorrowFail::InsufficientCollateral);
                }
                LOW_RECEIPT_CAPACITY => {
                    low_now = true;
                }
                _ => {}
            };
            let receipt_id = transfer.prev_receipt_id + 1;
            transfer.prev_receipt_id = receipt_id;
            PooledReceipt {
                receipt_id,
                unlocked_fee: U256::zero(),
            }
        } else {
            let receipts = &mut transfer.receipt_cache;
            let index = rng().gen_range(0..receipts.len());
            receipts.swap_remove(index)
        };

        // Technically we don't need the mutable borrow from here on out.
        // If we ever need to unlock more concurency when these are locked
        // it would be possible to split out the remainder of this method.

        // Write the data in the official receipt that gets sent over the wire.
        // This is: [vector_transfer_id, fee, receipt_id, signature]
        // That this math cannot overflow otherwise the transfer would have run out of collateral.
        let mut commitment = Vec::with_capacity(BORROWED_RECEIPT_LEN);
        let fee = receipt.unlocked_fee + locked_fee;
        commitment.extend_from_slice(&transfer.vector_transfer_id);
        commitment.extend_from_slice(&to_be_bytes(fee));
        commitment.extend_from_slice(&receipt.receipt_id.to_be_bytes());

        // Engineering in any kind of replay protection like as afforded by EIP-712 is
        // unnecessary, because the signer key needs to be unique per app. It is a straightforward
        // extension from there to also say that the signer key should be globally unique and
        // not sign any messages that are not for the app. Since there are no other structs
        // to sign, there are no possible collisions.
        //
        // The part of the message that needs to be signed sn the fee and receipt id only.
        let signed_data = &commitment[FEE_RANGE.start..RECEIPT_ID_RANGE.end];
        let message = Message::from_slice(&hash_bytes(signed_data)).unwrap();

        let signature = SIGNER.sign_recoverable(&message, &transfer.signer);
        let (recovery_id, signature) = signature.serialize_compact();
        let recovery_id = match recovery_id.to_i32() {
            0 => 27,
            1 => 28,
            27 => 27,
            28 => 28,
            _ => panic!("Invalid recovery id"),
        };
        commitment.extend_from_slice(&signature);
        commitment.push(recovery_id);

        // Extend with the unlocked fee, which is necessary to return collateral
        // in the case of failure.
        commitment.extend_from_slice(&to_be_bytes(receipt.unlocked_fee));

        debug_assert_eq!(BORROWED_RECEIPT_LEN, commitment.len());

        let low_collateral_warning = low_before != low_now;
        if low_collateral_warning {
            // Setting this to 0 ensures that if collateral is returned
            // to the transfer
            transfer.low_collateral_threshold = U256::zero()
        }

        Ok(ReceiptBorrow {
            commitment,
            low_collateral_warning,
        })
    }

    pub fn release(&mut self, bytes: &[u8], status: QueryStatus) {
        assert_eq!(bytes.len(), BORROWED_RECEIPT_LEN);
        let vector_transfer_id: Bytes32 = bytes[VECTOR_TRANSFER_ID_RANGE].try_into().unwrap();

        // Try to find the transfer. If there is no transfer, it means it's been uninstalled.
        // In that case, drop the receipt.
        let transfer = if let Some(transfer) = self.transfer_by_id_mut(&vector_transfer_id) {
            transfer
        } else {
            return;
        };

        let fee = U256::from_big_endian(&bytes[FEE_RANGE]);
        let unlocked_fee = U256::from_big_endian(&bytes[UNLOCKED_FEE_RANGE]);
        let locked_fee = fee - unlocked_fee;

        let mut receipt = PooledReceipt {
            unlocked_fee,
            receipt_id: ReceiptID::from_be_bytes(bytes[RECEIPT_ID_RANGE].try_into().unwrap()),
        };

        let funds_destination = match status {
            QueryStatus::Failure => &mut transfer.remaining_collateral,
            QueryStatus::Success => &mut receipt.unlocked_fee,
            // If we don't know what happened (eg: timeout) we don't
            // know what the Indexer perceives the state of the receipt to
            // be. Rather than arguing about it (eg: sync protocol) just
            // never use this receipt again by dropping it here. This
            // does not change any security invariants. We also do not reclaim
            // the collateral until transfer uninstall.
            //
            // FIXME: HACK! Until we have a ZKP and can remove trust from the
            // gateway return Unknown receipts to the pool as if they failed.
            // Fail is the most likely outcome. Dropping the receipt would
            // have the effect of syncing later. Until we have the ZKP though,
            // the threshold where receipts deplete is much lower than we would
            // like. Once there is a ZKP and trust is removed from the Gateway
            // this behavior needs to be restored because the indexer is expected
            // to verify that the balance is increasing and in some cases they
            // will not agree (eg: query was served but the network failed)
            // See also 9e55d5ff-8b49-4af4-9c1d-4137ac4cffb4
            // QueryStatus::Unknown => return,
            QueryStatus::Unknown => &mut transfer.remaining_collateral,
        };

        *funds_destination += locked_fee;
        transfer.receipt_cache.push(receipt);
    }
}

fn to_be_bytes(value: U256) -> Bytes32 {
    let mut result = Bytes32::default();
    value.to_big_endian(&mut result);
    result
}

fn hash_bytes(bytes: &[u8]) -> Bytes32 {
    use tiny_keccak::Hasher;
    let mut hasher = tiny_keccak::Keccak::v256();
    hasher.update(&bytes);
    let mut output = Bytes32::default();
    hasher.finalize(&mut output);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::*;

    #[track_caller]
    fn assert_failed_borrow(pool: &mut ReceiptPool, fee: impl Into<U256>) {
        let receipt = pool.commit(fee.into());
        assert_eq!(receipt, Err(BorrowFail::InsufficientCollateral));
    }

    #[track_caller]
    fn assert_collateral_equals(pool: &ReceiptPool, expect: impl Into<U256>) {
        let expect = expect.into();
        let mut collateral = U256::zero();
        for transfer in pool.transfers.iter() {
            collateral += transfer.remaining_collateral;
        }
        assert_eq!(expect, collateral);
    }

    #[track_caller]
    fn assert_successful_borrow(pool: &mut ReceiptPool, fee: impl Into<U256>) -> Vec<u8> {
        pool.commit(U256::from(fee.into()))
            .expect("Should have sufficient collateral")
            .commitment
    }

    // Has 2 transfers which if together could pay for a receipt, but separately cannot.
    //
    // It occurs to me that the updated receipts could be an array - allow this to succeed by committing
    // to partial fees across multiple transfers. This is likely not worth the complexity, but could be
    // a way to drain all transfers down to 0 remaining collateral. If this gets added and this test fails,
    // then the transfer selection test will no longer be effective. See also 460f4588-66f3-4aa3-9715-f2da8cac20b7
    #[test]
    pub fn cannot_share_collateral_across_transfers() {
        let mut pool = ReceiptPool::new();

        pool.add_transfer(test_signer(), 50.into(), bytes32(1));
        pool.add_transfer(test_signer(), 25.into(), bytes32(2));

        assert_failed_borrow(&mut pool, 60);
    }

    // Simple happy-path case of paying for requests in a loop.
    #[test]
    pub fn can_pay_for_requests() {
        let mut pool = ReceiptPool::new();
        pool.add_transfer(test_signer(), 60.into(), bytes32(1));

        for i in 1..=10 {
            let borrow = assert_successful_borrow(&mut pool, i);
            pool.release(&borrow, QueryStatus::Success);
            // Verify that we have unlocked all the fees
            let unlocked: u32 = (0..=i).sum();
            assert_eq!(U256::from(unlocked), pool.known_unlocked_fees());
        }

        // Should run out of collateral here.
        assert_failed_borrow(&mut pool, 6);
    }

    // If the transfers aren't selected optimally, then this will fail to pay for the full set.
    #[test]
    pub fn selects_best_transfers() {
        // Assumes fee cannot be split across transfers.
        // See also 460f4588-66f3-4aa3-9715-f2da8cac20b7
        let mut pool = ReceiptPool::new();

        pool.add_transfer(test_signer(), 4.into(), bytes32(1));
        pool.add_transfer(test_signer(), 3.into(), bytes32(2));
        pool.add_transfer(test_signer(), 1.into(), bytes32(3));
        pool.add_transfer(test_signer(), 2.into(), bytes32(4));
        pool.add_transfer(test_signer(), 2.into(), bytes32(5));
        pool.add_transfer(test_signer(), 1.into(), bytes32(6));
        pool.add_transfer(test_signer(), 3.into(), bytes32(7));
        pool.add_transfer(test_signer(), 4.into(), bytes32(8));

        assert_successful_borrow(&mut pool, 2);
        assert_successful_borrow(&mut pool, 4);
        assert_successful_borrow(&mut pool, 3);
        assert_successful_borrow(&mut pool, 1);
        assert_successful_borrow(&mut pool, 2);
        assert_successful_borrow(&mut pool, 3);
        assert_successful_borrow(&mut pool, 1);
        assert_successful_borrow(&mut pool, 4);

        assert_failed_borrow(&mut pool, 1);
    }

    // Any uninstalled transfer cannot be used to pay for queries.
    #[test]
    fn removed_transfer_cannot_pay() {
        let mut pool = ReceiptPool::new();
        pool.add_transfer(test_signer(), 10.into(), bytes32(2));
        pool.add_transfer(test_signer(), 3.into(), bytes32(1));

        pool.remove_transfer(&bytes32(2));

        assert_failed_borrow(&mut pool, 5);
    }

    // Tests modes for returning collateral
    #[test]
    fn collateral_return() {
        let mut pool = ReceiptPool::new();

        pool.add_transfer(test_signer(), 10.into(), bytes32(2));

        let borrow3 = assert_successful_borrow(&mut pool, 3);
        assert_collateral_equals(&pool, 7);

        let borrow2 = assert_successful_borrow(&mut pool, 2);
        assert_collateral_equals(&pool, 5);

        pool.release(&borrow3, QueryStatus::Failure);
        assert_collateral_equals(&pool, 8);

        let _borrow4 = assert_successful_borrow(&mut pool, 4);
        assert_collateral_equals(&pool, 4);

        pool.release(&borrow2, QueryStatus::Success);
        assert_collateral_equals(&pool, 4);

        // Unknown status is temporarily "wrong"
        // See also 9e55d5ff-8b49-4af4-9c1d-4137ac4cffb4
        //pool.release(&borrow4, QueryStatus::Unknown);
        //assert_collateral_equals(&pool, 4);
    }
}
