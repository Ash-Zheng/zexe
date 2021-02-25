use rand::Rng;
use std::time::Instant;

use algebra_core::{
    msm::VariableBaseMSM, AffineCurve, PairingEngine, PrimeField, ProjectiveCurve, UniformRand,
    Zero,
};

use crate::{r1cs_to_qap::R1CStoQAP, Parameters, Proof, Vec};

use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};

use ff_fft::{cfg_into_iter, cfg_iter, EvaluationDomain};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

pub fn create_random_proof<E, C, D, R>(
    circuit: C,
    params: &Parameters<E>,
    rng: &mut R,
) -> Result<Proof<E>, SynthesisError>
where
    E: PairingEngine,
    C: ConstraintSynthesizer<E::Fr>,
    D: EvaluationDomain<E::Fr>,
    R: Rng,
{
    let r = E::Fr::rand(rng);
    let s = E::Fr::rand(rng);

    create_proof::<E, C, D>(circuit, params, r, s)
}

pub fn create_proof_no_zk<E, C, D>(
    circuit: C,
    params: &Parameters<E>,
) -> Result<Proof<E>, SynthesisError>
where
    E: PairingEngine,
    C: ConstraintSynthesizer<E::Fr>,
    D: EvaluationDomain<E::Fr>,
{
    create_proof::<E, C, D>(circuit, params, E::Fr::zero(), E::Fr::zero())
}

pub fn create_proof<E, C, D>(
    circuit: C,
    params: &Parameters<E>,
    r: E::Fr,
    s: E::Fr,
) -> Result<Proof<E>, SynthesisError>
where
    E: PairingEngine,
    C: ConstraintSynthesizer<E::Fr>,
    D: EvaluationDomain<E::Fr>,
{
    // let mut file = OpenOptions::new().append(true).open("result.txt").expect(
    //     "cannot open file");
    // file.write_all("shallownet_optimized_pedersen_mnist:".as_bytes()).expect("write failed");

    let prover_time = start_timer!(|| "Groth16::Prover");
    let cs = ConstraintSystem::new_ref();

    // Synthesize the circuit.
    let begin = Instant::now();
    let synthesis_time = start_timer!(|| "Constraint synthesis");
    circuit.generate_constraints(cs.clone())?;
    debug_assert!(cs.is_satisfied().unwrap());
    end_timer!(synthesis_time);
    let end = Instant::now();
    println!("Zheng Constraint synthesis time {:?}", end.duration_since(begin));
    // let time = end.duration_since(begin);
    // let s_time = time.as_secs_f32().to_string();
    // file.write_all("\n Constraint synthesis time:".as_bytes()).expect("write failed");
    // file.write_all(s_time.as_bytes()).expect("write failed");

    // println!("Zheng CS befor inline{:?}", cs);

    let begin = Instant::now();
    let lc_time = start_timer!(|| "Inlining LCs");

    let begin_check = Instant::now();
    cs.parallel_inline_check();
    let end_check = Instant::now();
    println!("Zheng parallel check time {:?}", end_check.duration_since(begin_check));

    // println!("Zheng cs before inline {:?}", cs);
    cs.inline_all_lcs();
    // println!("Zheng cs after inline {:?}", cs);
    end_timer!(lc_time);
    let end = Instant::now();
    println!("Zheng Inlining LCs time {:?}", end.duration_since(begin));
    // let time = end.duration_since(begin);
    // let s_time = time.as_secs_f32().to_string();
    // file.write_all("\n Inlining LCs time:".as_bytes()).expect("write failed");
    // file.write_all(s_time.as_bytes()).expect("write failed");

    // println!("Zheng CS after inline{:?}", cs);

    let begin = Instant::now();
    let witness_map_time = start_timer!(|| "R1CS to QAP witness map");
    let h = R1CStoQAP::witness_map::<E, D>(cs.clone())?;
    end_timer!(witness_map_time);
    let end = Instant::now();
    println!("Zheng R1CS to QAP witness map time {:?}", end.duration_since(begin));
    // let time = end.duration_since(begin);
    // let s_time = time.as_secs_f32().to_string();
    // file.write_all("\n R1CS to QAP witness map time:".as_bytes()).expect("write failed");
    // file.write_all(s_time.as_bytes()).expect("write failed");

    let prover = cs.borrow().unwrap();
    let input_assignment = prover.instance_assignment[1..]
        .into_iter()
        .map(|s| s.into_repr())
        .collect::<Vec<_>>();

    let aux_assignment = cfg_iter!(prover.witness_assignment)
        .map(|s| s.into_repr())
        .collect::<Vec<_>>();

    println!("\nzheng input_assignment.len():{:?}", prover.instance_assignment.len());
    println!("\nzheng aux_assignment.len():{:?}", prover.witness_assignment.len());

    drop(prover);

    let assignment = [&input_assignment[..], &aux_assignment[..]].concat();

    let h_assignment = cfg_into_iter!(h).map(|s| s.into_repr()).collect::<Vec<_>>();

    // Compute A
    let begin = Instant::now();
    let a_acc_time = start_timer!(|| "Compute A");
    let a_query = params.get_a_query_full()?;
    let r_g1 = params.delta_g1.mul(r);
    let g_a = calculate_coeff(r_g1, a_query, params.vk.alpha_g1, &assignment);
    end_timer!(a_acc_time);
    let end = Instant::now();
    println!("Zheng Compute A time {:?}", end.duration_since(begin));
    // let time = end.duration_since(begin);
    // let s_time = time.as_secs_f32().to_string();
    // file.write_all("\n Compute A time:".as_bytes()).expect("write failed");
    // file.write_all(s_time.as_bytes()).expect("write failed");

    // Compute B in G1 if needed
    let g1_b = if r != E::Fr::zero() {
        let begin = Instant::now();
        let b_g1_acc_time = start_timer!(|| "Compute B in G1");
        let s_g1 = params.delta_g1.mul(s);
        let b_query = params.get_b_g1_query_full()?;

        let g1_b = calculate_coeff(s_g1, b_query, params.beta_g1, &assignment);

        end_timer!(b_g1_acc_time);
        let end = Instant::now();
        println!("Zheng Compute B in G1 time {:?}", end.duration_since(begin));
        // let time = end.duration_since(begin);
        // let s_time = time.as_secs_f32().to_string();
        // file.write_all("\n Compute B in G1 time:".as_bytes()).expect("write failed");
        // file.write_all(s_time.as_bytes()).expect("write failed");

        g1_b
    } else {
        E::G1Projective::zero()
    };

    // Compute B in G2
    let begin = Instant::now();
    let b_g2_acc_time = start_timer!(|| "Compute B in G2");
    let b_query = params.get_b_g2_query_full()?;
    let s_g2 = params.vk.delta_g2.mul(s);
    let g2_b = calculate_coeff(s_g2, b_query, params.vk.beta_g2, &assignment);
    drop(assignment);
    end_timer!(b_g2_acc_time);
    let end = Instant::now();
    println!("Zheng Compute B in G2 time {:?}", end.duration_since(begin));
    // let time = end.duration_since(begin);
    // let s_time = time.as_secs_f32().to_string();
    // file.write_all("\n Compute B in G2 time:".as_bytes()).expect("write failed");
    // file.write_all(s_time.as_bytes()).expect("write failed");

    // Compute C
    let begin = Instant::now();
    let c_acc_time = start_timer!(|| "Compute C");

    let h_query = params.get_h_query_full()?;
    let h_acc = VariableBaseMSM::multi_scalar_mul(&h_query, &h_assignment);
    drop(h_assignment);
    let l_aux_source = params.get_l_query_full()?;
    let l_aux_acc = VariableBaseMSM::multi_scalar_mul(l_aux_source, &aux_assignment);
    drop(aux_assignment);

    let s_g_a = g_a.mul(s);
    let r_g1_b = g1_b.mul(r);
    let r_s_delta_g1 = params.delta_g1.into_projective().mul(r).mul(s);

    let mut g_c = s_g_a;
    g_c += &r_g1_b;
    g_c -= &r_s_delta_g1;
    g_c += &l_aux_acc;
    g_c += &h_acc;
    end_timer!(c_acc_time);
    let end = Instant::now();
    println!("Zheng Compute C time {:?}", end.duration_since(begin));
    // let time = end.duration_since(begin);
    // let s_time = time.as_secs_f32().to_string();
    // file.write_all("\n Compute C time:".as_bytes()).expect("write failed");
    // file.write_all(s_time.as_bytes()).expect("write failed");

    end_timer!(prover_time);

    Ok(Proof {
        a: g_a.into_affine(),
        b: g2_b.into_affine(),
        c: g_c.into_affine(),
    })
}

fn calculate_coeff<G: AffineCurve>(
    initial: G::Projective,
    query: &[G],
    vk_param: G,
    assignment: &[<G::ScalarField as PrimeField>::BigInt],
) -> G::Projective {
    let el = query[0];
    let acc = VariableBaseMSM::multi_scalar_mul(&query[1..], assignment);

    let mut res = initial;
    res.add_assign_mixed(&el);
    res += &acc;
    res.add_assign_mixed(&vk_param);

    res
}
