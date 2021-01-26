use std::time::Instant;

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

pub struct Receipt {
    i: u128,
    payment_amount: u128,
}

// TODO:
pub fn create_proof(receipts: &[Receipt]) {
    //let pminusonedivtwo: Fr = Fr::modulus_minus_one_div_two().into();

    // TODO: Checked math
    let sum: u128 = receipts.iter().map(|r| r.payment_amount).sum();
    let cs = ConstraintSystem::<Fr>::new_ref();
    let sum_i = var(&cs, &sum, Input);

    let mut sum_v = var(&cs, &0, Constant);
    for (i, receipt) in receipts.iter().enumerate() {
        // TODO: Fill in the "None" values
        let receipt_id = var(&cs, &(i as u128), Constant);
        let amount = var(&cs, &receipt.payment_amount, Witness);
        // TODO: Sign receipt_id, amount
        sum_v = sum_v + amount;
    }
    sum_i.enforce_equal(&sum_v).unwrap();

    cs.finalize();
    assert!(cs.is_satisfied().unwrap());
    dbg!(cs.num_constraints());
    dbg!(cs.num_witness_variables());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn prove_sum() {
        // Create test data
        let data: Vec<u128> = (100..12000).collect();
        let data: Vec<Receipt> = data
            .iter()
            .enumerate()
            .map(|(i, &payment_amount)| Receipt {
                i: i as u128,
                payment_amount,
            })
            .collect();

        // Create the proof on different sizes
        let mut prev = 0;
        for len in 0..data.len() {
            let x = (len as f64).log2() as u32;
            if x != prev || len == 1 {
                prev = x;
                dbg!(len);
                let start = Instant::now();
                create_proof(&data[0..len]);
                dbg!(Instant::now() - start);
            }
        }

        panic!("End")
    }
}
