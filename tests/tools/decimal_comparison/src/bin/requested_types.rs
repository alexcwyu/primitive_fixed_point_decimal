//! Requested concrete types over common exact six-decimal inputs.
use current::{ConstScaleFpdec, OobScaleFpdec};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashSet},
    fmt::Debug,
    hint::black_box,
    mem::size_of,
    sync::Arc,
    time::Instant,
};

const INPUT_DP: u32 = 6;
const PAIRS: usize = 1024;
const OPS: [&str; 4] = ["add", "sub", "mul", "div"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema: u32,
    seed: u64,
    description: String,
    cases: Vec<RawCase>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCase {
    workload: String,
    a: String,
    b: String,
    trunc: [String; 4],
    native: BTreeMap<String, [String; 4]>,
    native_trunc: BTreeMap<String, [String; 4]>,
}
struct Case {
    a: i128,
    b: i128,
    trunc: [i128; 4],
    native: BTreeMap<u32, [i128; 4]>,
    native_trunc: BTreeMap<u32, [i128; 4]>,
}
struct Group {
    workload: String,
    cases: Arc<Vec<Case>>,
}
fn oracle_map(raw: BTreeMap<String, [String; 4]>, keys: &[u32]) -> BTreeMap<u32, [i128; 4]> {
    let result: BTreeMap<_, _> = raw
        .into_iter()
        .map(|(dp, values)| {
            (
                dp.parse::<u32>().expect("invalid oracle dp"),
                values.map(|n| n.parse::<i128>().expect("invalid native oracle")),
            )
        })
        .collect();
    assert_eq!(
        result.keys().copied().collect::<Vec<_>>(),
        keys,
        "wrong native oracle scales"
    );
    result
}
fn load(path: &str) -> Vec<Group> {
    let f: Fixture = serde_json::from_slice(&std::fs::read(path).expect("cannot read fixture"))
        .expect("invalid fixture JSON");
    assert_eq!(f.schema, 2, "requested-types requires fixture schema2");
    assert!(
        !f.description.trim().is_empty(),
        "empty fixture description"
    );
    black_box(f.seed);
    let mut groups: BTreeMap<String, Vec<Case>> = BTreeMap::new();
    for c in f.cases {
        assert!(
            matches!(c.workload.as_str(), "positive" | "mixed_sign"),
            "invalid workload"
        );
        let a: i128 = c.a.parse().expect("invalid a mantissa");
        let b: i128 = c.b.parse().expect("invalid b mantissa");
        assert!(
            (500_000..=1_500_000).contains(&a.abs()) && (500_000..=1_500_000).contains(&b.abs()),
            "inputs must have magnitudes .5..1.5 at6dp"
        );
        if c.workload == "positive" {
            assert!(a > 0 && b > 0, "positive workload has negative input");
        }
        let trunc = c
            .trunc
            .map(|n| n.parse::<i128>().expect("invalid trunc oracle"));
        assert_eq!(trunc[0], a + b, "invalid addition oracle");
        assert_eq!(trunc[1], a - b, "invalid subtraction oracle");
        let native = oracle_map(c.native, &[6, 8, 10, 12, 18]);
        let native_trunc = oracle_map(c.native_trunc, &[8, 12]);
        groups.entry(c.workload).or_default().push(Case {
            a,
            b,
            trunc,
            native,
            native_trunc,
        });
    }
    assert_eq!(groups.len(), 2, "both workloads required");
    groups
        .into_iter()
        .map(|(workload, cases)| {
            assert_eq!(cases.len(), PAIRS, "1024 pairs required per workload");
            Group {
                workload,
                cases: Arc::new(cases),
            }
        })
        .collect()
}

trait Number: Debug + Sized + 'static {
    const VARIANT: &'static str;
    const DP: u32;
    const EXACT: bool = true;
    const NATIVE_TRUNCATED: bool = false;
    const VISIBILITY: &'static str = "compile-time";
    const API: &'static str =
        "native operators; matched checked arithmetic then checked_round_ext(6,TowardsZero)";
    fn input(mantissa6: i128) -> Self;
    fn eval<const OP: u8, const MATCHED: bool>(a: &Self, b: &Self, runtime_scale: i32) -> Self;
    fn raw(&self) -> i128;
    fn observed<const MATCHED: bool>(&self) -> i128 {
        self.raw()
            / if MATCHED {
                10_i128.pow(Self::DP - INPUT_DP)
            } else {
                1
            }
    }
    fn scaled_value<const MATCHED: bool>(&self) -> f64 {
        self.observed::<MATCHED>() as f64
    }
}
macro_rules! const_number {
    ($lib:ident, $int:ty, $dp:literal, $name:literal) => {
        impl Number for $lib::ConstScaleFpdec<$int, $dp> {
            const VARIANT: &'static str = $name;
            const DP: u32 = $dp;
            fn input(m: i128) -> Self {
                Self::from_mantissa(
                    (m * 10_i128.pow($dp - INPUT_DP))
                        .try_into()
                        .expect("input exceeds native mantissa"),
                )
            }
            #[inline(always)]
            fn eval<const OP: u8, const MATCHED: bool>(a: &Self, b: &Self, _: i32) -> Self {
                if MATCHED {
                    let value = match OP {
                        0 => a.checked_add(*b),
                        1 => a.checked_sub(*b),
                        2 => a.checked_mul_ext::<$int, $dp, $dp>(*b, $lib::Rounding::TowardsZero),
                        3 => a.checked_div_ext::<$int, $dp, $dp>(*b, $lib::Rounding::TowardsZero),
                        _ => unreachable!(),
                    }
                    .expect("checked const arithmetic failed");
                    value
                        .checked_round_ext(6, $lib::Rounding::TowardsZero)
                        .expect("checked const quantization failed")
                } else {
                    match OP {
                        0 => *a + *b,
                        1 => *a - *b,
                        2 => *a * *b,
                        3 => *a / *b,
                        _ => unreachable!(),
                    }
                }
            }
            fn raw(&self) -> i128 {
                self.mantissa() as i128
            }
        }
    };
}
const_number!(current, i32, 6, "current_const_i32_scale6");
const_number!(current, i64, 8, "current_const_i64_scale8");
const_number!(current, i128, 8, "current_const_i128_scale8");
const_number!(current, i128, 10, "current_const_i128_scale10");
const_number!(current, i128, 12, "current_const_i128_scale12");
const_number!(current, i128, 18, "current_const_i128_scale18");
const_number!(baseline, i128, 18, "baseline_const_i128_scale18");

#[repr(transparent)]
#[derive(Debug)]
struct RuntimeOob<T, const DP: u32>(T);
macro_rules! oob_number {
    ($lib:ident, $int:ty, $dp:literal, $name:literal) => {
        impl Number for RuntimeOob<$lib::OobScaleFpdec<$int>, $dp> {
            const VARIANT: &'static str = $name;
            const DP: u32 = $dp;
            const VISIBILITY: &'static str = "runtime";
            const API: &'static str = "native add/sub operators and default-round checked mul/div; matched checked arithmetic then checked_round_diff_ext(runtime_scale-6,TowardsZero)";
            fn input(m: i128) -> Self { Self($lib::OobScaleFpdec::from_mantissa((m * 10_i128.pow($dp - INPUT_DP)).try_into().expect("input exceeds native mantissa"))) }
            #[inline(always)]
            fn eval<const OP: u8, const MATCHED: bool>(a: &Self, b: &Self, scale: i32) -> Self {
                Self(if MATCHED {
                    let value = match OP {
                        0 => a.0.checked_add(b.0), 1 => a.0.checked_sub(b.0),
                        2 => a.0.checked_mul_ext(b.0, scale, $lib::Rounding::TowardsZero),
                        3 => a.0.checked_div_ext(b.0, -scale, $lib::Rounding::TowardsZero),
                        _ => unreachable!(),
                    }.expect("checked OOB arithmetic failed");
                    value.checked_round_diff_ext(scale - 6, $lib::Rounding::TowardsZero).expect("checked OOB quantization failed")
                } else { match OP {
                    0 => a.0 + b.0, 1 => a.0 - b.0,
                    2 => a.0.checked_mul(b.0, scale).expect("native OOB multiply failed"),
                    3 => a.0.checked_div(b.0, -scale).expect("native OOB divide failed"), _ => unreachable!(),
                } })
            }
            fn raw(&self) -> i128 { self.0.mantissa() as i128 }
        }
    };
}
oob_number!(current, i64, 8, "current_oob_i64_scale8");
oob_number!(current, i128, 12, "current_oob_i128_scale12");
oob_number!(current, i128, 18, "current_oob_i128_scale18");
oob_number!(baseline, i128, 18, "baseline_oob_i128_scale18");

macro_rules! fixdec_number {
    ($ty:ty, $dp:literal, $name:literal) => {
        impl Number for $ty {
            const VARIANT: &'static str = $name;
            const DP: u32 = $dp;
            const NATIVE_TRUNCATED: bool = true;
            const API: &'static str = "native operators truncate at native scale; matched checked arithmetic and round_dp_with_strategy(6,ToZero), division checked_div_rounded(6,ToZero)";
            fn input(m: i128) -> Self { Self::try_with_scale(m.try_into().expect("input exceeds fixdec mantissa"), INPUT_DP).expect("inexact fixdec input") }
            #[inline(always)]
            fn eval<const OP: u8, const MATCHED: bool>(a: &Self, b: &Self, _: i32) -> Self {
                if MATCHED {
                    if OP == 3 { return a.checked_div_rounded(*b, 6, fixdec::RoundingStrategy::ToZero).expect("checked fixdec divide failed"); }
                    let value = match OP { 0 => a.checked_add(*b), 1 => a.checked_sub(*b), 2 => a.checked_mul(*b), _ => unreachable!() }.expect("checked fixdec arithmetic failed");
                    // Both precision reductions truncate toward zero, so composing them is safe.
                    value.round_dp_with_strategy(6, fixdec::RoundingStrategy::ToZero)
                } else { match OP { 0 => *a + *b, 1 => *a - *b, 2 => *a * *b, 3 => *a / *b, _ => unreachable!() } }
            }
            fn raw(&self) -> i128 { self.to_raw() as i128 }
        }
    };
}
fixdec_number!(fixdec::D64, 8, "fixdec_D64_native8");
fixdec_number!(fixdec::D96, 12, "fixdec_D96_native12");
impl Number for f64 {
    const VARIANT: &'static str = "f64";
    const DP: u32 = 6;
    const EXACT: bool = false;
    const API: &'static str = "native IEEE754 operators; matched finite assertion and (result*1e6).trunc()/1e6; native dp6 is diagnostic extraction only";
    fn input(m: i128) -> Self {
        m as f64 / 1_000_000.0
    }
    #[inline(always)]
    fn eval<const OP: u8, const MATCHED: bool>(a: &Self, b: &Self, _: i32) -> Self {
        let value = match OP {
            0 => a + b,
            1 => a - b,
            2 => a * b,
            3 => a / b,
            _ => unreachable!(),
        };
        if MATCHED {
            assert!(value.is_finite(), "nonfinite f64 result");
            (value * 1_000_000.0).trunc() / 1_000_000.0
        } else {
            value
        }
    }
    fn raw(&self) -> i128 {
        assert!(self.is_finite(), "nonfinite f64 result");
        (*self * 1_000_000.0).trunc() as i128
    }
    fn scaled_value<const MATCHED: bool>(&self) -> f64 {
        *self * 1_000_000.0
    }
}

#[derive(Serialize)]
struct Meta {
    id: String,
    variant: &'static str,
    workload: String,
    dp: u32,
    mode: &'static str,
    op: &'static str,
    bytes: usize,
    exact_decimal: bool,
    representation_dp: Option<u32>,
    target_visibility: &'static str,
    input_dp: u32,
    native_api: &'static str,
    oracle: String,
}
#[derive(Serialize)]
struct Example {
    pair: usize,
    a: String,
    b: String,
    expected: String,
    actual: String,
    native_result: String,
}
#[derive(Serialize)]
struct Validation {
    checked_pairs: usize,
    mismatches: usize,
    max_error_units: f64,
    examples: Vec<Example>,
}
struct Cell {
    meta: Meta,
    timed: Box<dyn Fn(u64) -> u128>,
    validate: Box<dyn Fn() -> Validation>,
}
fn reference<T: Number, const OP: u8, const MATCHED: bool>(c: &Case) -> i128 {
    if MATCHED || !T::EXACT {
        c.trunc[OP as usize]
    } else if T::NATIVE_TRUNCATED {
        c.native_trunc[&T::DP][OP as usize]
    } else {
        c.native[&T::DP][OP as usize]
    }
}
fn validate<T: Number, const OP: u8, const MATCHED: bool>(
    pairs: &[(T, T)],
    cases: &[Case],
) -> Validation {
    let scale = black_box(T::DP as i32);
    let mut result = Validation {
        checked_pairs: 0,
        mismatches: 0,
        max_error_units: 0.0,
        examples: vec![],
    };
    for (pair, ((a, b), c)) in pairs.iter().zip(cases).enumerate() {
        let value = T::eval::<OP, MATCHED>(black_box(a), black_box(b), scale);
        let actual = value.observed::<MATCHED>();
        let expected = reference::<T, OP, MATCHED>(c);
        result.checked_pairs += 1;
        let error = if T::EXACT {
            actual.abs_diff(expected) as f64
        } else {
            (value.scaled_value::<MATCHED>() - expected as f64).abs()
        };
        result.max_error_units = result.max_error_units.max(error);
        if actual != expected {
            result.mismatches += 1;
            if result.examples.len() < 3 {
                result.examples.push(Example {
                    pair,
                    a: c.a.to_string(),
                    b: c.b.to_string(),
                    expected: expected.to_string(),
                    actual: actual.to_string(),
                    native_result: format!("{value:?}"),
                });
            }
        }
    }
    result
}
#[inline(never)]
fn timed<T: Number, const OP: u8, const MATCHED: bool>(pairs: &[(T, T)], batches: u64) -> u128 {
    // Catalog scales identify the cases; this barrier keeps OOB arithmetic truly runtime.
    let scale = black_box(T::DP as i32);
    let start = Instant::now();
    for _ in 0..batches {
        for (a, b) in pairs {
            black_box(T::eval::<OP, MATCHED>(black_box(a), black_box(b), scale));
        }
    }
    start.elapsed().as_nanos()
}
fn cell<T: Number, const OP: u8, const MATCHED: bool>(
    group: &Group,
    pairs: Arc<Vec<(T, T)>>,
) -> Cell {
    let mode = if MATCHED { "matched" } else { "native" };
    let dp = if MATCHED { INPUT_DP } else { T::DP };
    let cases = Arc::clone(&group.cases);
    let validation_pairs = Arc::clone(&pairs);
    Cell {
        meta: Meta {
            id: format!(
                "requested-types/{}/{}/dp{}/{}/{}",
                T::VARIANT,
                group.workload,
                dp,
                mode,
                OPS[OP as usize]
            ),
            variant: T::VARIANT,
            workload: group.workload.clone(),
            dp,
            mode,
            op: OPS[OP as usize],
            bytes: size_of::<T>(),
            exact_decimal: T::EXACT,
            representation_dp: if T::EXACT { Some(T::DP) } else { None },
            target_visibility: T::VISIBILITY,
            input_dp: INPUT_DP,
            native_api: T::API,
            oracle: if MATCHED || !T::EXACT {
                "trunc at6dp".into()
            } else if T::NATIVE_TRUNCATED {
                format!("native_trunc raw at{}dp", T::DP)
            } else {
                format!("native nearest-away raw at{}dp", T::DP)
            },
        },
        timed: Box::new(move |batches| timed::<T, OP, MATCHED>(&pairs, batches)),
        validate: Box::new(move || validate::<T, OP, MATCHED>(&validation_pairs, &cases)),
    }
}
fn register<T: Number>(group: &Group, result: &mut Vec<Cell>) {
    let pairs = Arc::new(
        group
            .cases
            .iter()
            .map(|c| (T::input(c.a), T::input(c.b)))
            .collect(),
    );
    macro_rules! add {
        ($op:literal) => {
            result.push(cell::<T, $op, false>(group, Arc::clone(&pairs)));
            result.push(cell::<T, $op, true>(group, Arc::clone(&pairs)));
        };
    }
    add!(0);
    add!(1);
    add!(2);
    add!(3);
}
fn cells(groups: &[Group]) -> Vec<Cell> {
    let mut result = vec![];
    for group in groups {
        register::<RuntimeOob<OobScaleFpdec<i64>, 8>>(group, &mut result);
        register::<RuntimeOob<OobScaleFpdec<i128>, 12>>(group, &mut result);
        register::<RuntimeOob<OobScaleFpdec<i128>, 18>>(group, &mut result);
        register::<RuntimeOob<baseline::OobScaleFpdec<i128>, 18>>(group, &mut result);
        register::<ConstScaleFpdec<i32, 6>>(group, &mut result);
        register::<ConstScaleFpdec<i64, 8>>(group, &mut result);
        register::<ConstScaleFpdec<i128, 8>>(group, &mut result);
        register::<ConstScaleFpdec<i128, 10>>(group, &mut result);
        register::<ConstScaleFpdec<i128, 12>>(group, &mut result);
        register::<ConstScaleFpdec<i128, 18>>(group, &mut result);
        register::<baseline::ConstScaleFpdec<i128, 18>>(group, &mut result);
        register::<f64>(group, &mut result);
        register::<fixdec::D64>(group, &mut result);
        register::<fixdec::D96>(group, &mut result);
    }
    assert_eq!(
        result
            .iter()
            .map(|c| c.meta.id.as_str())
            .collect::<HashSet<_>>()
            .len(),
        result.len(),
        "duplicate cell ids"
    );
    result
}
fn emit(meta: &Meta, fields: serde_json::Value) {
    let mut value = serde_json::to_value(meta).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    println!("{value}");
}
fn bench(cells: &mut [Cell], round: usize, target_ms: f64) {
    assert!(
        target_ms.is_finite() && target_ms > 0.0,
        "invalid TARGET_MS"
    );
    let target_ns = (target_ms * 1_000_000.0).ceil() as u128;
    let rotate = round.wrapping_mul(131) % cells.len();
    cells.rotate_left(rotate);
    if round % 2 == 1 {
        cells.reverse();
    }
    for (order, c) in cells.iter().enumerate() {
        (c.timed)(1);
        let calibration_ns = (c.timed)(1).max(1);
        let mut batches = u64::try_from(target_ns.div_ceil(calibration_ns))
            .expect("duration too large")
            .max(1);
        let mut elapsed = (c.timed)(batches);
        while elapsed < target_ns {
            batches = batches.checked_mul(2).expect("iteration overflow");
            elapsed = (c.timed)(batches);
        }
        let iterations = batches
            .checked_mul(PAIRS as u64)
            .expect("iteration overflow");
        emit(
            &c.meta,
            json!({"round":round,"order":order,"wall_ns":elapsed,"iterations":iterations,"ns_per_op":elapsed as f64 / iterations as f64,"target_ms":target_ms,"warmup_pairs":PAIRS,"calibration_pairs":PAIRS,"calibration_ns":calibration_ns}),
        );
    }
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert!(
        args.len() >= 3,
        "usage: requested_types (list|validate) FIXTURES | bench FIXTURES ROUND TARGET_MS"
    );
    let mut cells = cells(&load(&args[2]));
    match args[1].as_str() {
        "list" => {
            assert_eq!(args.len(), 3);
            println!(
                "{}",
                serde_json::to_string(&cells.iter().map(|c| &c.meta).collect::<Vec<_>>()).unwrap()
            );
        }
        "validate" => {
            assert_eq!(args.len(), 3);
            let mut failures = 0;
            for c in cells {
                let result = (c.validate)();
                failures += usize::from(c.meta.exact_decimal && result.mismatches != 0);
                emit(&c.meta, serde_json::to_value(result).unwrap());
            }
            assert_eq!(failures, 0, "exact-decimal cells disagree with oracle");
        }
        "bench" => {
            assert_eq!(args.len(), 5);
            bench(
                &mut cells,
                args[3].parse().expect("invalid ROUND"),
                args[4].parse().expect("invalid TARGET_MS"),
            );
        }
        _ => panic!("unknown command"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Case {
        Case {
            a: 1_000_001,
            b: 500_000,
            trunc: [1_500_001, 500_001, 500_000, 2_000_002],
            native: BTreeMap::from([
                (6, [1_500_001, 500_001, 500_001, 2_000_002]),
                (8, [150_000_100, 50_000_100, 50_000_050, 200_000_200]),
                (
                    10,
                    [15_000_010_000, 5_000_010_000, 5_000_005_000, 20_000_020_000],
                ),
                (
                    12,
                    [
                        1_500_001_000_000,
                        500_001_000_000,
                        500_000_500_000,
                        2_000_002_000_000,
                    ],
                ),
                (
                    18,
                    [
                        1_500_001_000_000_000_000,
                        500_001_000_000_000_000,
                        500_000_500_000_000_000,
                        2_000_002_000_000_000_000,
                    ],
                ),
            ]),
            native_trunc: BTreeMap::from([
                (8, [150_000_100, 50_000_100, 50_000_050, 200_000_200]),
                (
                    12,
                    [
                        1_500_001_000_000,
                        500_001_000_000,
                        500_000_500_000,
                        2_000_002_000_000,
                    ],
                ),
            ]),
        }
    }
    fn check_case_oracles<T: Number>(c: &Case) {
        let (a, b, scale) = (T::input(c.a), T::input(c.b), black_box(T::DP as i32));
        macro_rules! check {
            ($op:literal) => {
                assert_eq!(
                    T::eval::<$op, false>(&a, &b, scale).observed::<false>(),
                    reference::<T, $op, false>(c)
                );
                assert_eq!(
                    T::eval::<$op, true>(&a, &b, scale).observed::<true>(),
                    reference::<T, $op, true>(c)
                );
            };
        }
        check!(0);
        check!(1);
        check!(2);
        check!(3);
    }
    #[test]
    fn native_oracles_keep_representation_precision() {
        let c = sample();
        type C6 = ConstScaleFpdec<i32, 6>;
        type C8 = ConstScaleFpdec<i64, 8>;
        let a6 = C6::input(c.a);
        let b6 = C6::input(c.b);
        let a8 = C8::input(c.a);
        let b8 = C8::input(c.b);
        assert_eq!(
            C6::eval::<2, false>(&a6, &b6, 6).raw(),
            reference::<C6, 2, false>(&c)
        );
        assert_eq!(
            C8::eval::<2, false>(&a8, &b8, 8).raw(),
            reference::<C8, 2, false>(&c)
        );
        assert_eq!(reference::<C6, 2, false>(&c), 500_001);
        assert_eq!(reference::<C8, 2, false>(&c), 50_000_050);
        assert_eq!(C6::eval::<2, true>(&a6, &b6, 6).observed::<true>(), 500_000);
        assert_eq!(C8::eval::<2, true>(&a8, &b8, 8).observed::<true>(), 500_000);
        assert_eq!(reference::<fixdec::D64, 2, false>(&c), 50_000_050);
        check_case_oracles::<ConstScaleFpdec<i32, 6>>(&c);
        check_case_oracles::<ConstScaleFpdec<i64, 8>>(&c);
        check_case_oracles::<ConstScaleFpdec<i128, 8>>(&c);
        check_case_oracles::<ConstScaleFpdec<i128, 10>>(&c);
        check_case_oracles::<ConstScaleFpdec<i128, 12>>(&c);
        check_case_oracles::<ConstScaleFpdec<i128, 18>>(&c);
        check_case_oracles::<RuntimeOob<OobScaleFpdec<i64>, 8>>(&c);
        check_case_oracles::<RuntimeOob<OobScaleFpdec<i128>, 12>>(&c);
        check_case_oracles::<RuntimeOob<OobScaleFpdec<i128>, 18>>(&c);
        check_case_oracles::<baseline::ConstScaleFpdec<i128, 18>>(&c);
        check_case_oracles::<RuntimeOob<baseline::OobScaleFpdec<i128>, 18>>(&c);
        check_case_oracles::<fixdec::D64>(&c);
        check_case_oracles::<fixdec::D96>(&c);
    }
    #[test]
    fn registry_has_original_types_and_both_baseline18_controls() {
        let groups = vec![
            Group {
                workload: "positive".into(),
                cases: Arc::new(vec![sample()]),
            },
            Group {
                workload: "mixed_sign".into(),
                cases: Arc::new(vec![sample()]),
            },
        ];
        let rows = cells(&groups);
        assert_eq!(rows.len(), 224);
        assert_eq!(
            rows.iter()
                .map(|c| c.meta.variant)
                .collect::<HashSet<_>>()
                .len(),
            14
        );
        for dp in [8, 10, 12, 18] {
            assert!(rows.iter().any(
                |c| c.meta.variant == format!("current_const_i128_scale{dp}")
                    && c.meta.representation_dp == Some(dp)
                    && c.meta.target_visibility == "compile-time"
            ));
        }
        for dp in [12, 18] {
            assert!(rows
                .iter()
                .any(|c| c.meta.variant == format!("current_oob_i128_scale{dp}")
                    && c.meta.representation_dp == Some(dp)
                    && c.meta.target_visibility == "runtime"));
        }
        for family in ["const", "oob"] {
            assert!(rows.iter().any(|c| c.meta.variant
                == format!("baseline_{family}_i128_scale18")
                && c.meta.representation_dp == Some(18)
                && c.meta.target_visibility
                    == if family == "oob" {
                        "runtime"
                    } else {
                        "compile-time"
                    }));
        }
        assert!(rows
            .iter()
            .filter(|c| c.meta.mode == "matched")
            .all(|c| c.meta.dp == 6));
        assert_eq!(rows.iter().filter(|c| !c.meta.exact_decimal).count(), 16);
    }
}
