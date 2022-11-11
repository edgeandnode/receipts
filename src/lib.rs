#[cfg(feature = "receiver")]
pub mod receiver;
#[cfg(feature = "sender")]
pub mod sender;

pub use primitive_types::U256;

pub type Bytes32 = [u8; 32];
pub type Address = [u8; 20];
// This can't be [u8; 16] because then the length would collide with the
// transfer implementation which uses Bytes32 (TransferId) + u32 (ReceiptId) = 36 bytes
// and this would have been Address (AllocationId) + 16 = 36 bytes.
pub type ReceiptId = [u8; 15];
pub type Signature = [u8; 65];
