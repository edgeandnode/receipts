/*
# ZKP
ark-gm17 = { git = "https://github.com/arkworks-rs/gm17", features = [ "r1cs" ] }
ark-r1cs-std = { git = "https://github.com/arkworks-rs/r1cs-std" }
ark-bn254 = { git = "https://github.com/arkworks-rs/curves" }
*/

use std::time::Instant;

//use ark_ed_on_bls12_381::{EdwardsProjective as JubJub, FrParameters};
use ark_ff::Fp256;
use ark_r1cs_std::alloc::AllocationMode::*;
use ark_r1cs_std::eq::EqGadget;
use ark_r1cs_std::prelude::AllocationMode;
use ark_r1cs_std::{alloc::AllocVar, fields::fp::FpVar};
use ark_relations::r1cs::{ConstraintSystem, ConstraintSystemRef};
use ark_test_curves::bls12_381::{Fr, FrParameters};

// ark_ed_on_bls12_381

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
        let amount = var(&cs, &receipt.payment_amount, Witness);
        sum_v = sum_v + amount;
    }
    sum_i.enforce_equal(&sum_v).unwrap();

    cs.finalize();
    assert!(cs.is_satisfied().unwrap());
    dbg!(cs.num_constraints());
    dbg!(cs.num_witness_variables());
}

/*
// Variations:
// Assert each row = to a constant
// Assert mul rows = (some number) where signer only uses primes (starting with 2, not 1)
// The above are probably dumb... we actually just need to feed each row as a const into
// the signer and not 'assert' anything.
fn ordering_proof(data: &[u128]) -> CS {
    let cs = ConstraintSystem::<Fr>::new_ref();

    let mut data = data.iter();
    let initial = data.next().unwrap();
    let mut prev = witness_in(initial, &cs);
    for elem in data {
        let next = witness_in(elem, &cs);
        prev.enforce_cmp(&next, Ordering::Less, false).unwrap();
        prev = next;
    }
    cs.finalize();
    dbg!(cs.is_satisfied().unwrap());
    dbg!(cs.num_constraints());
    dbg!(cs.num_witness_variables());
    cs
}
*/

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
