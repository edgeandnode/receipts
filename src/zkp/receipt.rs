pub type ID = u32;
pub type Payment = u128;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Receipt {
    pub id: ID,
    pub payment_amount: Payment,
    pub signature: (),
}

impl Receipt {
    // This takes about 4s on my laptop:
    // pub const MAX_ID: ID = 16383;
    // This is effectively instant:
    pub const MAX_ID: ID = 2048;

    pub const fn null(id: ID) -> Self {
        Receipt {
            id,
            payment_amount: 0,
            signature: (),
        }
    }
}
