use crate::*;

pub struct ReceiptInfo {
    pub id: ReceiptId,
    pub allocation: Address,
    pub fees: U256,
    pub signature: Signature,
}

pub fn validate_receipt(receipt_hex: &str) -> Option<ReceiptInfo> {
    // TODO: (Security) Additional validations will be required to remove trust from the Gateway.

    let mut bytes = [0u8; 130];
    hex::decode_to_slice(receipt_hex.trim_start_matches("0x"), &mut bytes).ok()?;

    todo!()
}
