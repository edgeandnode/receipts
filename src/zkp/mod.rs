#![allow(dead_code)]
mod iter;
use iter::*;
mod receipt;
use receipt::*;

use ark_ed_on_bls12_381::{EdwardsProjective as JubJub, Fr, FrParameters};
use ark_ff::Fp256;
use ark_r1cs_std::alloc::AllocationMode::*;
use ark_r1cs_std::eq::EqGadget;
use ark_r1cs_std::prelude::AllocationMode;
use ark_r1cs_std::{alloc::AllocVar, fields::fp::FpVar};
use ark_relations::r1cs::{ConstraintSystem, ConstraintSystemRef};

type FP = Fp256<FrParameters>;
type CS = ConstraintSystemRef<FP>;

fn var(cs: &CS, value: &u128, mode: AllocationMode) -> FpVar<FP> {
    let value: Fr = (*value).into();
    FpVar::<Fr>::new_variable(cs.clone(), || Ok(value), mode).unwrap()
}

pub fn create_proof(receipts: Vec<Receipt>) -> Result<(), Error> {
    let receipts = rows(receipts)?;
    //let pminusonedivtwo: Fr = Fr::modulus_minus_one_div_two().into();

    // TODO: Checked math
    let mut sum = 0;
    let cs = ConstraintSystem::<Fr>::new_ref();
    let mut sum_v = var(&cs, &0, Constant);
    for receipt in receipts {
        sum += receipt.payment_amount;

        let receipt_id = var(&cs, &(receipt.id as u128), Constant);
        let amount = var(&cs, &receipt.payment_amount, Witness);
        // TODO: Sign receipt_id, amount
        sum_v = sum_v + amount;
    }
    let sum_i = var(&cs, &sum, Input);
    sum_i.enforce_equal(&sum_v).unwrap();

    cs.finalize();
    assert!(cs.is_satisfied().unwrap());
    dbg!(cs.num_constraints());
    dbg!(cs.num_witness_variables());

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    pub fn prove_sum() {
        // Create test data
        let data: Vec<u128> = (0..100).collect();
        let data: Vec<Receipt> = data
            .iter()
            .enumerate()
            .map(|(i, &payment_amount)| Receipt {
                id: i as u32,
                payment_amount,
                signature: (),
            })
            .collect();

        let start = Instant::now();
        create_proof(data).unwrap();
        dbg!(Instant::now() - start);
    }
}
