use crate::prelude::*;
use secp256k1::{Message, PublicKey, SecretKey};
use std::fmt;
use tiny_keccak::{Hasher, Keccak};

#[derive(Debug)]
pub enum VoucherError {
    InvalidData,
    InvalidSignature,
    UnorderedReceipts,
    NoValue,
}

impl std::error::Error for VoucherError {}

impl fmt::Display for VoucherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidData => write!(f, "Invalid receipts data"),
            Self::InvalidSignature => write!(f, "Receipts are not signed for the given allocation"),
            Self::UnorderedReceipts => write!(f, "Unordered receipts"),
            Self::NoValue => write!(f, "Receipts have no value"),
        }
    }
}

const PAYMENT_AMOUNT_RANGE: Range = next_range::<U256>(0..0);
const RECEIPT_ID_RANGE: Range = next_range::<ReceiptId>(PAYMENT_AMOUNT_RANGE);
const SIGNATURE_RANGE: Range = next_range::<Signature>(RECEIPT_ID_RANGE);
const SIZE: usize = SIGNATURE_RANGE.end; // 112 bytes, last I checked.

// TODO: (Performance)
// At 112 bytes each 1M receipts costs 106MiB.
// This payload size is concerning, so it may be useful to allow for this to be
// broken up and aggregated in chunks. For example, submit 20k receipts at a time
// at 2.1MiB per request and get back a signed message that includes
// (min_receipt_id, amount). By including the min_receipt_id we can still enforce
// uniqueness and roll this up incrementally.
// Alternatively we could support reading data from a stream. But, most APIs these
// days make that difficult.
//
/// Security: The voucher_signer must be dedicated to this purpose, hold no funds,
/// and sign no other messages except with this method. Similarly, the allocation
/// signer must only sign allocations and serve no other purpose and hold no funds.
/// One exception is that they may be the same signer. They are allowed to be different
/// in case we want to rotate the voucher_signer and keep old receipts intact. Having
/// them be the same signer is ok only because they sign messages of different lengths.
pub fn receipts_to_voucher(
    allocation_id: &Address,
    allocation_signer: &PublicKey,
    voucher_signer: &SecretKey,
    data: &[u8],
) -> Result<Vec<u8>, VoucherError> {
    // Data must be an array of fixed size elements
    // containing receipts.
    if data.len() % SIZE != 0 {
        return Err(VoucherError::InvalidData);
    }

    // To keep track of uniqueness. 0 will never be used as
    // a receipt id.
    let mut prev_receipt_id: ReceiptId = ReceiptId::default();

    // Keep track of value unlocked for signing voucher.
    let mut total = U256::zero();

    // Iterate over each receipt
    for chunk in data.chunks_exact(SIZE) {
        // Verify the receipts are sorted and ascending.
        // This also verifies their uniqueness.
        // Unwrap is safe because we know the chunk has the exact amount of data required
        let receipt_id: ReceiptId = (&chunk[RECEIPT_ID_RANGE]).try_into().unwrap();
        if !(prev_receipt_id < receipt_id) {
            return Err(VoucherError::UnorderedReceipts);
        }
        prev_receipt_id = receipt_id;

        let signature = &chunk[SIGNATURE_RANGE];

        let signature = secp256k1::Signature::from_compact(&signature[..64])
            .map_err(|_| VoucherError::InvalidData)?;

        // Create the signed message from the receipt data.
        // Allocationid is "untrusted" and kept separate from the receipt data.
        // This also de-duplicates it in the message.
        let mut hasher = Keccak::v256();
        hasher.update(allocation_id);
        hasher.update(&chunk[PAYMENT_AMOUNT_RANGE.start..RECEIPT_ID_RANGE.end]);
        let mut message = Bytes32::default();
        hasher.finalize(&mut message);

        let message = Message::from_slice(&message).unwrap();

        SECP256K1
            .verify(&message, &signature, allocation_signer)
            .map_err(|_| VoucherError::InvalidSignature)?;

        let this_amount = U256::from_big_endian(&chunk[PAYMENT_AMOUNT_RANGE]);

        total = total.saturating_add(this_amount);
    }

    // The contract will revert if this is 0
    if total == U256::zero() {
        return Err(VoucherError::NoValue);
    }

    // Write the commitment that can be brought on-chain
    let mut message = Vec::new();
    message.extend_from_slice(allocation_id);
    message.extend_from_slice(&to_be_bytes(total));
    let signature = sign(&message, voucher_signer);
    message.extend_from_slice(&signature);
    Ok(message)
}