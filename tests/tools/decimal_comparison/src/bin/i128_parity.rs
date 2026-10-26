//! Same-input, same-output TowardZero arithmetic for the frozen baseline, current, and D96.
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

const PAIRS: usize = 1024;
const OPS: [&str; 4] = ["add", "sub", "mul", "div"];
const WORKLOADS: [&str; 6] = [
    "small_positive",
    "small_mixed_sign",
    "amounts_positive",
    "amounts_mixed_sign",
    "rate_positive",
    "rate_mixed_sign",
];

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
    dp: u32,
    workload: String,
    a: String,
    b: String,
    trunc: [String; 4],
}
struct Case {
    a: i128,
    b: i128,
    trunc: [i128; 4],
}
struct Group {
    dp: u32,
    workload: String,
    cases: Arc<Vec<Case>>,
}

fn load(path: &str) -> Vec<Group> {
    let f: Fixture = serde_json::from_slice(&std::fs::read(path).expect("cannot read fixture"))
        .expect("invalid fixture JSON");
    assert_eq!(f.schema, 3, "i128 parity requires fixture schema3");
    assert!(
        !f.description.trim().is_empty(),
        "empty fixture description"
    );
    black_box(f.seed);
    let mut groups: BTreeMap<(u32, String), Vec<Case>> = BTreeMap::new();
    for c in f.cases {
        assert!(
            [8, 10, 12].contains(&c.dp) && WORKLOADS.contains(&c.workload.as_str()),
            "unsupported scale/workload"
        );
        let a: i128 = c.a.parse().expect("invalid a mantissa");
        let b: i128 = c.b.parse().expect("invalid b mantissa");
        let (a_min, a_max, b_min, b_max) = if c.workload.starts_with("small_") {
            (500_000, 1_500_000, 500_000, 1_500_000)
        } else if c.workload.starts_with("amounts_") {
            (1_000_000, 100_000_000_000_000, 10_000, 10_000_000)
        } else {
            (1_000_000, 100_000_000_000_000, 10, 10_000)
        };
        assert!(
            (a_min..=a_max).contains(&a.abs()) && (b_min..=b_max).contains(&b.abs()),
            "input outside agreed workload"
        );
        if c.workload.ends_with("_positive") {
            assert!(a > 0 && b > 0, "negative input in positive workload");
        }
        let trunc = c
            .trunc
            .map(|n| n.parse::<i128>().expect("invalid target oracle"));
        let factor = 10_i128.pow(c.dp - 6);
        assert_eq!(trunc[0], (a + b) * factor, "bad add oracle");
        assert_eq!(trunc[1], (a - b) * factor, "bad sub oracle");
        groups
            .entry((c.dp, c.workload))
            .or_default()
            .push(Case { a, b, trunc });
    }
    assert_eq!(
        groups.len(),
        18,
        "all six workloads and three scales required"
    );
    groups
        .into_iter()
        .map(|((dp, workload), cases)| {
            assert_eq!(cases.len(), PAIRS, "1024 pairs required per cell");
            Group {
                dp,
                workload,
                cases: Arc::new(cases),
            }
        })
        .collect()
}

trait Number: Debug + Sized + 'static {
    const VARIANT: &'static str;
    const VISIBILITY: &'static str;
    const API: &'static str;
    fn input(mantissa6: i128, dp: u32) -> Self;
    fn eval<const OP: u8>(a: &Self, b: &Self, runtime_scale: i32) -> Self;
    fn extract(&self, dp: u32) -> i128;
    fn representation_dp(dp: u32) -> u32 {
        dp
    }
}
// Share panic metadata so identical checked loops can merge across implementations.
#[cold]
#[inline(never)]
fn arithmetic_failed() -> ! {
    panic!("checked arithmetic failed")
}
#[inline(always)]
fn checked_value<T>(value: Option<T>) -> T {
    value.unwrap_or_else(|| arithmetic_failed())
}
macro_rules! const_number {
    ($lib:ident) => {
        impl<const S: i32> Number for $lib::ConstScaleFpdec<i128, S> {
            const VARIANT: &'static str = concat!(stringify!($lib), "_const");
            const VISIBILITY: &'static str = "compile-time";
            const API: &'static str =
                "checked add/sub; checked_mul_ext/checked_div_ext(TowardsZero), result scale S";
            fn input(m: i128, dp: u32) -> Self {
                assert_eq!(dp as i32, S);
                Self::from_mantissa(m.checked_mul(10_i128.pow(dp - 6)).expect("input overflow"))
            }
            #[inline(always)]
            fn eval<const OP: u8>(a: &Self, b: &Self, _: i32) -> Self {
                checked_value(match OP {
                    0 => a.checked_add(*b),
                    1 => a.checked_sub(*b),
                    2 => a.checked_mul_ext::<i128, S, S>(*b, $lib::Rounding::TowardsZero),
                    3 => a.checked_div_ext::<i128, S, S>(*b, $lib::Rounding::TowardsZero),
                    _ => unreachable!(),
                })
            }
            fn extract(&self, _: u32) -> i128 {
                self.mantissa()
            }
        }
    };
}
const_number!(baseline);
const_number!(current);
macro_rules! oob_number {
    ($lib:ident) => {
        impl Number for $lib::OobScaleFpdec<i128> {
            const VARIANT: &'static str = concat!(stringify!($lib), "_oob");
            const VISIBILITY: &'static str = "runtime";
            const API: &'static str = "checked add/sub; checked_mul_ext/checked_div_ext(TowardsZero), opaque runtime scale";
            fn input(m: i128, dp: u32) -> Self { Self::from_mantissa(m.checked_mul(10_i128.pow(dp - 6)).expect("input overflow")) }
            #[inline(always)]
            fn eval<const OP: u8>(a: &Self, b: &Self, scale: i32) -> Self {
                checked_value(match OP {
                    0 => a.checked_add(*b), 1 => a.checked_sub(*b),
                    2 => a.checked_mul_ext(*b, scale, $lib::Rounding::TowardsZero),
                    3 => a.checked_div_ext(*b, -scale, $lib::Rounding::TowardsZero), _ => unreachable!(),
                })
            }
            fn extract(&self, _: u32) -> i128 { self.mantissa() }
        }
    };
}
oob_number!(baseline);
oob_number!(current);
#[repr(transparent)]
#[derive(Debug)]
struct D96At<const S: i32, const NATIVE: bool = false>(fixdec::D96);
impl<const S: i32, const NATIVE: bool> Number for D96At<S, NATIVE> {
    const VARIANT: &'static str = if NATIVE { "D96Native" } else { "D96" };
    const VISIBILITY: &'static str = "compile-time";
    const API: &'static str = if NATIVE {
        "checked add/sub; checked native12 multiply/divide directly at S12, public ToZero reduction only at S8/10 (all intermediates fit this workload)"
    } else {
        "checked add/sub; checked native12 multiply directly at S12, public ToZero reduction only at S8/10; checked_div_rounded(S,ToZero)"
    };
    fn input(m: i128, dp: u32) -> Self {
        assert_eq!(dp as i32, S);
        Self(fixdec::D96::try_with_scale(m, 6).expect("inexact D96 input"))
    }
    #[inline(always)]
    fn eval<const OP: u8>(a: &Self, b: &Self, _: i32) -> Self {
        Self(match OP {
            // Exact6dp inputs put add/sub outputs on every requested target grid.
            0 => checked_value(a.0.checked_add(b.0)),
            1 => checked_value(a.0.checked_sub(b.0)),
            2 => {
                let value = checked_value(a.0.checked_mul(b.0));
                if S == 12 {
                    value
                } else {
                    value.round_dp_with_strategy(S as u8, fixdec::RoundingStrategy::ToZero)
                }
            }
            3 => {
                if NATIVE {
                    let value = checked_value(a.0.checked_div(b.0));
                    if S == 12 {
                        value
                    } else {
                        value.round_dp_with_strategy(S as u8, fixdec::RoundingStrategy::ToZero)
                    }
                } else {
                    checked_value(a.0.checked_div_rounded(b.0, S as u8, fixdec::RoundingStrategy::ToZero))
                }
            }
            _ => unreachable!(),
        })
    }
    fn extract(&self, dp: u32) -> i128 {
        self.0.to_raw() / 10_i128.pow(12 - dp)
    }
    fn representation_dp(_: u32) -> u32 {
        12
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
    representation_dp: u32,
    target_visibility: &'static str,
    input_dp: u32,
    native_api: &'static str,
}
#[derive(Serialize)]
struct Example {
    pair: usize,
    a: String,
    b: String,
    expected: String,
    actual: String,
}
#[derive(Serialize)]
struct Validation {
    checked_pairs: usize,
    mismatches: usize,
    max_error_units: u128,
    examples: Vec<Example>,
}
struct Cell {
    meta: Meta,
    timed: Box<dyn Fn(u64) -> u128>,
    validate: Box<dyn Fn() -> Validation>,
}
fn validate<T: Number, const OP: u8>(pairs: &[(T, T)], cases: &[Case], dp: u32) -> Validation {
    let scale = black_box(dp as i32);
    let mut result = Validation {
        checked_pairs: 0,
        mismatches: 0,
        max_error_units: 0,
        examples: vec![],
    };
    for (pair, ((a, b), c)) in pairs.iter().zip(cases).enumerate() {
        let value = T::eval::<OP>(black_box(a), black_box(b), scale);
        let actual = value.extract(dp);
        let expected = c.trunc[OP as usize];
        result.checked_pairs += 1;
        result.max_error_units = result.max_error_units.max(actual.abs_diff(expected));
        if actual != expected {
            result.mismatches += 1;
            if result.examples.len() < 3 {
                result.examples.push(Example {
                    pair,
                    a: c.a.to_string(),
                    b: c.b.to_string(),
                    expected: expected.to_string(),
                    actual: actual.to_string(),
                });
            }
        }
    }
    result
}
#[inline(never)]
fn timed<T: Number, const OP: u8>(pairs: &[(T, T)], batches: u64, dp: u32) -> u128 {
    let scale = black_box(dp as i32);
    let start = Instant::now();
    for _ in 0..batches {
        for (a, b) in pairs {
            black_box(T::eval::<OP>(black_box(a), black_box(b), scale));
        }
    }
    start.elapsed().as_nanos()
}
fn cell<T: Number, const OP: u8>(group: &Group, pairs: Arc<Vec<(T, T)>>) -> Cell {
    let dp = group.dp;
    let cases = Arc::clone(&group.cases);
    let vpairs = Arc::clone(&pairs);
    Cell {
        meta: Meta {
            id: format!(
                "i128-parity/{}/{}/dp{}/matched/{}",
                T::VARIANT,
                group.workload,
                dp,
                OPS[OP as usize]
            ),
            variant: T::VARIANT,
            workload: group.workload.clone(),
            dp,
            mode: "matched",
            op: OPS[OP as usize],
            bytes: size_of::<T>(),
            exact_decimal: true,
            representation_dp: T::representation_dp(dp),
            target_visibility: T::VISIBILITY,
            input_dp: 6,
            native_api: T::API,
        },
        timed: Box::new(move |batches| timed::<T, OP>(&pairs, batches, dp)),
        validate: Box::new(move || validate::<T, OP>(&vpairs, &cases, dp)),
    }
}
fn register<T: Number>(group: &Group, result: &mut Vec<Cell>) {
    let pairs = Arc::new(
        group
            .cases
            .iter()
            .map(|c| (T::input(c.a, group.dp), T::input(c.b, group.dp)))
            .collect(),
    );
    macro_rules! add {
        ($op:literal) => {
            result.push(cell::<T, $op>(group, Arc::clone(&pairs)));
        };
    }
    add!(0);
    add!(1);
    add!(2);
    add!(3);
}
fn register_static<const S: i32>(group: &Group, result: &mut Vec<Cell>) {
    register::<baseline::ConstScaleFpdec<i128, S>>(group, result);
    register::<current::ConstScaleFpdec<i128, S>>(group, result);
    register::<D96At<S>>(group, result);
    register::<D96At<S, true>>(group, result);
}
fn cells(groups: &[Group]) -> Vec<Cell> {
    let mut result = vec![];
    for group in groups {
        match group.dp {
            8 => register_static::<8>(group, &mut result),
            10 => register_static::<10>(group, &mut result),
            12 => register_static::<12>(group, &mut result),
            _ => unreachable!(),
        }
        register::<current::OobScaleFpdec<i128>>(group, &mut result);
        register::<baseline::OobScaleFpdec<i128>>(group, &mut result);
    }
    assert_eq!(
        result
            .iter()
            .map(|c| c.meta.id.as_str())
            .collect::<HashSet<_>>()
            .len(),
        result.len(),
        "duplicate cell id"
    );
    result
}
fn emit(meta: &Meta, fields: serde_json::Value) {
    let mut row = serde_json::to_value(meta).unwrap();
    row.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    println!("{row}");
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
            json!({"round":round,"order":order,"wall_ns":elapsed,"iterations":iterations,"ns_per_op":elapsed as f64/iterations as f64,"target_ms":target_ms,"warmup_pairs":PAIRS,"calibration_pairs":PAIRS,"calibration_ns":calibration_ns}),
        );
    }
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert!(
        args.len() >= 3,
        "usage: i128_parity (list|validate) FIXTURES | bench FIXTURES ROUND TARGET_MS"
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
                let v = (c.validate)();
                failures += usize::from(v.mismatches != 0);
                emit(&c.meta, serde_json::to_value(v).unwrap());
            }
            assert_eq!(failures, 0, "exact cells disagree with oracle");
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
    fn assert_checked_failure<T: Number, const OP: u8>(a: T, b: T, scale: i32) {
        let error = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            T::eval::<OP>(&a, &b, scale)
        }))
        .expect_err("checked arithmetic unexpectedly succeeded");
        let message = error.downcast_ref::<&str>().copied()
            .or_else(|| error.downcast_ref::<String>().map(String::as_str));
        assert_eq!(message, Some("checked arithmetic failed"));
    }
    fn check_pfpd_failures<const S: i32>() {
        macro_rules! check {
            ($ty:ty) => {{
                let make = <$ty>::from_mantissa;
                assert_checked_failure::<$ty, 0>(make(i128::MAX), make(1), S);
                assert_checked_failure::<$ty, 1>(make(i128::MIN), make(1), S);
                assert_checked_failure::<$ty, 2>(make(i128::MAX), make(i128::MAX), S);
                assert_checked_failure::<$ty, 3>(make(1), make(0), S);
            }};
        }
        check!(baseline::ConstScaleFpdec<i128, S>);
        check!(current::ConstScaleFpdec<i128, S>);
        check!(baseline::OobScaleFpdec<i128>);
        check!(current::OobScaleFpdec<i128>);
    }
    fn check_d96_failures<const S: i32, const NATIVE: bool>() {
        let max = || D96At::<S, NATIVE>(fixdec::D96::MAX);
        let min = || D96At::<S, NATIVE>(fixdec::D96::MIN);
        let one_unit = || D96At::<S, NATIVE>(fixdec::D96::try_with_scale(1, 12).unwrap());
        let zero = || D96At::<S, NATIVE>::input(0, S as u32);
        assert_checked_failure::<D96At<S, NATIVE>, 0>(max(), one_unit(), S);
        assert_checked_failure::<D96At<S, NATIVE>, 1>(min(), one_unit(), S);
        assert_checked_failure::<D96At<S, NATIVE>, 2>(max(), max(), S);
        assert_checked_failure::<D96At<S, NATIVE>, 3>(max(), zero(), S);
    }
    #[test]
    fn pfpd_overflow_and_zero_division_keep_shared_checked_failure() {
        check_pfpd_failures::<8>();
        check_pfpd_failures::<10>();
        check_pfpd_failures::<12>();
    }
    #[test]
    fn both_d96_paths_keep_shared_checked_failure() {
        check_d96_failures::<8, false>();
        check_d96_failures::<10, false>();
        check_d96_failures::<12, false>();
        check_d96_failures::<8, true>();
        check_d96_failures::<10, true>();
        check_d96_failures::<12, true>();
    }
    fn groups() -> Vec<Group> {
        [8, 10, 12]
            .into_iter()
            .flat_map(|dp| {
                WORKLOADS.into_iter().map(move |workload| {
                    let factor = 10_i128.pow(dp - 8);
                    Group {
                        dp,
                        workload: workload.into(),
                        cases: Arc::new(vec![Case {
                            a: 1_000_001,
                            b: 500_000,
                            trunc: [150_000_100, 50_000_100, 50_000_050, 200_000_200]
                                .map(|n| n * factor),
                        }]),
                    }
                })
            })
            .collect()
    }
    #[test]
    fn all_families_and_targets_match_same_exact_oracle() {
        let rows = cells(&groups());
        assert_eq!(rows.len(), 432);
        assert_eq!(
            rows.iter()
                .map(|c| c.meta.variant)
                .collect::<HashSet<_>>()
                .len(),
            6
        );
        assert_eq!(
            rows.iter()
                .filter(|c| c.meta.target_visibility == "runtime")
                .count(),
            144
        );
        for c in rows {
            assert_eq!((c.validate)().mismatches, 0, "{}", c.meta.id);
        }
    }
    fn check_division_paths<const S: i32>(a: i128, b: i128) {
        let left = D96At::<S>::input(a, S as u32);
        let right = D96At::<S>::input(b, S as u32);
        let native_left = D96At::<S, true>::input(a, S as u32);
        let native_right = D96At::<S, true>::input(b, S as u32);
        let expected = a * 10_i128.pow(S as u32) / b;
        assert_eq!(
            D96At::<S>::eval::<3>(&left, &right, S).extract(S as u32),
            expected
        );
        assert_eq!(
            D96At::<S, true>::eval::<3>(&native_left, &native_right, S).extract(S as u32),
            expected
        );
    }
    #[test]
    fn both_d96_division_paths_match_signed_truncation_and_large_rates() {
        for (a, b) in [
            (1_000_004, 1_280_000),
            (1_000_002, 1_048_576),
            (100_000_000_000_000, 10),
        ] {
            for (sa, sb) in [(1, 1), (-1, 1), (1, -1), (-1, -1)] {
                check_division_paths::<8>(a * sa, b * sb);
                check_division_paths::<10>(a * sa, b * sb);
                check_division_paths::<12>(a * sa, b * sb);
            }
        }
    }
}
