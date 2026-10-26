//! Independent operations over cached pairs. Parsing and extraction are untimed.
use bigdecimal::BigDecimal;
use fixed::types::{I32F32, I64F64};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashSet},
    hint::black_box,
    mem::size_of,
    sync::Arc,
    time::Instant,
};

const OPS: [&str; 4] = ["add", "sub", "mul", "div"];
const PAIRS: usize = 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema: u32,
    seed: u64,
    description: String,
    cases: Vec<FixtureCase>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureCase {
    dp: u32,
    workload: String,
    a: String,
    b: String,
    trunc: [String; 4],
    round: [String; 4],
}
#[derive(Clone)]
struct Case {
    a: i128,
    b: i128,
    trunc: [i128; 4],
    round: [i128; 4],
}
struct Group {
    dp: u32,
    workload: String,
    cases: Arc<Vec<Case>>,
}

fn fixture_groups(f: Fixture) -> Vec<Group> {
    assert_eq!(f.schema, 1, "unsupported fixture schema");
    assert!(
        !f.description.trim().is_empty(),
        "fixture description is empty"
    );
    black_box(f.seed);
    let mut groups: BTreeMap<(u32, String), Vec<Case>> = BTreeMap::new();
    for c in f.cases {
        assert!((6..=8).contains(&c.dp), "unsupported dp");
        assert!(
            matches!(c.workload.as_str(), "positive" | "mixed_sign"),
            "unsupported workload"
        );
        let a: i128 = c.a.parse().expect("invalid a mantissa");
        let b: i128 = c.b.parse().expect("invalid b mantissa");
        let scale = 10_i128.pow(c.dp);
        assert!(
            a.abs() < 1000 * scale && b.abs() <= 10 * scale,
            "fixture exceeds bounded workload"
        );
        assert!(
            a.abs() >= scale / 100 && b.abs() >= scale / 100,
            "fixture too small or divides by zero"
        );
        if c.workload == "positive" {
            assert!(
                a >= b && b > 0,
                "positive fixture would underflow unsigned subtraction"
            );
        }
        let trunc = c
            .trunc
            .map(|s| s.parse::<i128>().expect("invalid trunc oracle"));
        let round = c
            .round
            .map(|s| s.parse::<i128>().expect("invalid round oracle"));
        assert_eq!(trunc[0], a + b, "bad addition oracle");
        assert_eq!(trunc[1], a - b, "bad subtraction oracle");
        assert_eq!(
            round[..2],
            trunc[..2],
            "round oracle differs on exact add/sub"
        );
        groups
            .entry((c.dp, c.workload))
            .or_default()
            .push(Case { a, b, trunc, round });
    }
    assert_eq!(groups.len(), 6, "need both workloads at every dp");
    groups
        .into_iter()
        .map(|((dp, workload), cases)| {
            assert_eq!(
                cases.len(),
                PAIRS,
                "need exactly 1024 pairs per dp/workload"
            );
            Group {
                dp,
                workload,
                cases: Arc::new(cases),
            }
        })
        .collect()
}
fn load(path: &str) -> Vec<Group> {
    fixture_groups(
        serde_json::from_slice(&std::fs::read(path).expect("cannot read fixture file"))
            .expect("invalid fixture JSON"),
    )
}
fn decimal_text(m: i128, dp: u32) -> String {
    let scale = 10_i128.pow(dp);
    format!(
        "{}{}.{:0width$}",
        if m < 0 { "-" } else { "" },
        m.abs() / scale,
        m.abs() % scale,
        width = dp as usize
    )
}
// Only called after validation operations, never inside a timer.
fn decimal_mantissa(text: &str, dp: u32) -> i128 {
    let value: BigDecimal = text.parse().expect("nonfinite or invalid decimal result");
    let (coefficient, exponent) = value.with_scale(dp as i64).as_bigint_and_exponent();
    assert_eq!(exponent, dp as i64);
    coefficient
        .to_string()
        .parse()
        .expect("result exceeds i128 validation range")
}
struct Settings {
    dp: i32,
    float_factor: f64,
    fixed_factor: I64F64,
}
impl Settings {
    fn new(dp: i32) -> Self {
        Self {
            dp,
            float_factor: 10_f64.powi(dp),
            fixed_factor: I64F64::from_num(10_i64.pow(dp as u32)),
        }
    }
}
trait Number: Sized + 'static {
    const VARIANT: &'static str;
    const EXACT: bool = true;
    const UNSIGNED: bool = false;
    const NATIVE_ROUNDS: bool = false;
    const BASELINE: bool = false;
    const NATIVE_API: &'static str = "operators";
    const INTERMEDIATE: &'static str = "native representation";
    const TARGET_VISIBILITY: &'static str = "runtime";
    fn representation_dp(dp: u32) -> Option<u32> {
        Some(dp)
    }
    fn input(mantissa: i128, dp: u32) -> Self;
    fn eval<const OP: u8, const MATCHED: bool>(a: &Self, b: &Self, settings: &Settings) -> Self;
    fn extract(&self, dp: u32) -> i128;
    fn scaled_value(&self, dp: u32) -> f64 {
        self.extract(dp) as f64
    }
}

#[repr(transparent)]
struct KnownPrecision<T, const DP: u32>(T);
impl<T, const DP: u32> KnownPrecision<T, DP> {
    const SETTINGS: Settings = match DP {
        6 => Settings {
            dp: 6,
            float_factor: 1_000_000.0,
            fixed_factor: I64F64::from_bits(1_000_000_i128 << 64),
        },
        7 => Settings {
            dp: 7,
            float_factor: 10_000_000.0,
            fixed_factor: I64F64::from_bits(10_000_000_i128 << 64),
        },
        8 => Settings {
            dp: 8,
            float_factor: 100_000_000.0,
            fixed_factor: I64F64::from_bits(100_000_000_i128 << 64),
        },
        _ => panic!("unsupported known target"),
    };
}
impl<T: Number, const DP: u32> Number for KnownPrecision<T, DP> {
    const VARIANT: &'static str = T::VARIANT;
    const EXACT: bool = T::EXACT;
    const UNSIGNED: bool = T::UNSIGNED;
    const NATIVE_ROUNDS: bool = T::NATIVE_ROUNDS;
    const BASELINE: bool = T::BASELINE;
    const NATIVE_API: &'static str = T::NATIVE_API;
    const INTERMEDIATE: &'static str = T::INTERMEDIATE;
    const TARGET_VISIBILITY: &'static str = "compile-time";
    fn representation_dp(dp: u32) -> Option<u32> {
        T::representation_dp(dp)
    }
    fn input(m: i128, dp: u32) -> Self {
        assert_eq!(dp, DP, "known target differs from fixture");
        Self(T::input(m, dp))
    }
    #[inline(always)]
    fn eval<const OP: u8, const MATCHED: bool>(a: &Self, b: &Self, _: &Settings) -> Self {
        Self(T::eval::<OP, MATCHED>(&a.0, &b.0, &Self::SETTINGS))
    }
    fn extract(&self, dp: u32) -> i128 {
        self.0.extract(dp)
    }
    fn scaled_value(&self, dp: u32) -> f64 {
        self.0.scaled_value(dp)
    }
}
macro_rules! primitive_const {
    ($lib:ident, $int:ty, $unsigned:expr) => {
        impl<const DP: i32> Number for $lib::ConstScaleFpdec<$int, DP> {
            const VARIANT: &'static str = concat!(stringify!($lib), "_const_", stringify!($int));
            const UNSIGNED: bool = $unsigned;
            const NATIVE_ROUNDS: bool = true;
            const TARGET_VISIBILITY: &'static str = "compile-time";
            fn input(m: i128, _: u32) -> Self {
                Self::from_mantissa(m.try_into().expect("input mantissa does not fit"))
            }
            #[inline(always)]
            fn eval<const OP: u8, const MATCHED: bool>(a: &Self, b: &Self, _: &Settings) -> Self {
                if MATCHED {
                    match OP {
                        0 => a.checked_add(*b),
                        1 => a.checked_sub(*b),
                        2 => a.checked_mul_ext::<$int, DP, DP>(*b, $lib::Rounding::TowardsZero),
                        3 => a.checked_div_ext::<$int, DP, DP>(*b, $lib::Rounding::TowardsZero),
                        _ => unreachable!(),
                    }
                    .expect("matched primitive arithmetic failed")
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
            fn extract(&self, _: u32) -> i128 {
                self.mantissa().try_into().expect("result exceeds i128")
            }
        }
    };
}
macro_rules! primitive_oob {
    ($lib:ident, $int:ty, $unsigned:expr) => {
        impl Number for $lib::OobScaleFpdec<$int> {
            const VARIANT: &'static str = concat!(stringify!($lib), "_oob_", stringify!($int));
            const UNSIGNED: bool = $unsigned;
            const NATIVE_ROUNDS: bool = true;
            const NATIVE_API: &'static str =
                "add/sub operators; mul/div default-round checked APIs (no OOB decimal operators)";
            fn input(m: i128, _: u32) -> Self {
                Self::from_mantissa(m.try_into().expect("input mantissa does not fit"))
            }
            #[inline(always)]
            fn eval<const OP: u8, const MATCHED: bool>(a: &Self, b: &Self, s: &Settings) -> Self {
                if MATCHED {
                    match OP {
                        0 => a.checked_add(*b),
                        1 => a.checked_sub(*b),
                        2 => a.checked_mul_ext(*b, s.dp, $lib::Rounding::TowardsZero),
                        3 => a.checked_div_ext(*b, -s.dp, $lib::Rounding::TowardsZero),
                        _ => unreachable!(),
                    }
                    .expect("matched OOB arithmetic failed")
                } else {
                    match OP {
                        0 => *a + *b,
                        1 => *a - *b,
                        2 => a
                            .checked_mul(*b, s.dp)
                            .expect("native OOB multiplication failed"),
                        3 => a
                            .checked_div(*b, -s.dp)
                            .expect("native OOB division failed"),
                        _ => unreachable!(),
                    }
                }
            }
            fn extract(&self, _: u32) -> i128 {
                self.mantissa().try_into().expect("result exceeds i128")
            }
        }
    };
}
macro_rules! primitives {
    ($lib:ident) => {
        primitive_const!($lib, i64, false);
        primitive_const!($lib, u64, true);
        primitive_const!($lib, i128, false);
        primitive_const!($lib, u128, true);
        primitive_oob!($lib, i64, false);
        primitive_oob!($lib, u64, true);
        primitive_oob!($lib, i128, false);
        primitive_oob!($lib, u128, true);
    };
}
primitives!(current);
primitives!(upstream);
macro_rules! fixdec_number {
    ($ty:ty, $dp:expr) => {
        impl Number for $ty {
            const VARIANT: &'static str = concat!(stringify!($ty), "_native", stringify!($dp));
            fn representation_dp(_: u32) -> Option<u32> {
                Some($dp)
            }
            fn input(m: i128, dp: u32) -> Self {
                Self::try_with_scale(m.try_into().expect("input exceeds fixdec mantissa"), dp)
                    .expect("inexact fixdec input")
            }
            #[inline(always)]
            fn eval<const OP: u8, const MATCHED: bool>(a: &Self, b: &Self, s: &Settings) -> Self {
                if MATCHED {
                    match OP {
                        0 => a.checked_add(*b).expect("fixdec add overflow"),
                        1 => a.checked_sub(*b).expect("fixdec sub overflow"),
                        // Both reductions truncate toward zero, so truncating twice is safe.
                        2 => a
                            .checked_mul(*b)
                            .expect("fixdec mul overflow")
                            .round_dp_with_strategy(s.dp as u8, fixdec::RoundingStrategy::ToZero),
                        3 => a
                            .checked_div_rounded(*b, s.dp as u8, fixdec::RoundingStrategy::ToZero)
                            .expect("fixdec div failed"),
                        _ => unreachable!(),
                    }
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
            fn extract(&self, dp: u32) -> i128 {
                self.to_raw() as i128 / 10_i128.pow($dp - dp)
            }
        }
    };
}
fixdec_number!(fixdec::D64, 8);
fixdec_number!(fixdec::D96, 12);
impl Number for rust_decimal::Decimal {
    const VARIANT: &'static str = "rust_decimal";
    fn representation_dp(_: u32) -> Option<u32> {
        None
    }
    fn input(m: i128, dp: u32) -> Self {
        Self::from_i128_with_scale(m, dp)
    }
    #[inline(always)]
    fn eval<const OP: u8, const MATCHED: bool>(a: &Self, b: &Self, s: &Settings) -> Self {
        let value = if MATCHED {
            match OP {
                0 => a.checked_add(*b),
                1 => a.checked_sub(*b),
                2 => a.checked_mul(*b),
                3 => a.checked_div(*b),
                _ => unreachable!(),
            }
            .expect("rust_decimal arithmetic failed")
        } else {
            match OP {
                0 => *a + *b,
                1 => *a - *b,
                2 => *a * *b,
                3 => *a / *b,
                _ => unreachable!(),
            }
        };
        if MATCHED {
            value.trunc_with_scale(s.dp as u32)
        } else {
            value
        }
    }
    fn extract(&self, dp: u32) -> i128 {
        decimal_mantissa(&self.to_string(), dp)
    }
}
macro_rules! fastnum_number {
    ($ty:ty, $name:literal) => {
        impl Number for $ty {
            const VARIANT: &'static str = $name;
            const NATIVE_API: &'static str =
                "operators; matched uses trapping native methods and trunc_with_scale";
            fn representation_dp(_: u32) -> Option<u32> {
                None
            }
            fn input(m: i128, dp: u32) -> Self {
                decimal_text(m, dp).parse().expect("invalid fastnum input")
            }
            #[inline(always)]
            fn eval<const OP: u8, const MATCHED: bool>(a: &Self, b: &Self, s: &Settings) -> Self {
                let value = if MATCHED {
                    match OP {
                        0 => (*a).add(*b),
                        1 => (*a).sub(*b),
                        2 => (*a).mul(*b),
                        3 => (*a).div(*b),
                        _ => unreachable!(),
                    }
                } else {
                    match OP {
                        0 => *a + *b,
                        1 => *a - *b,
                        2 => *a * *b,
                        3 => *a / *b,
                        _ => unreachable!(),
                    }
                };
                if MATCHED {
                    value.trunc_with_scale(s.dp as i16)
                } else {
                    value
                }
            }
            fn extract(&self, dp: u32) -> i128 {
                assert!(self.is_finite(), "nonfinite fastnum result");
                decimal_mantissa(&self.to_string(), dp)
            }
        }
    };
}
fastnum_number!(fastnum::D128, "fastnum_D128");
fastnum_number!(fastnum::D256, "fastnum_D256");
impl Number for BigDecimal {
    const VARIANT: &'static str = "bigdecimal";
    const NATIVE_API: &'static str = "borrowed operators; matched adds with_scale_round(Down)";
    const INTERMEDIATE: &'static str =
        "division default100 significant digits in both modes; pinned API has no context division";
    fn representation_dp(_: u32) -> Option<u32> {
        None
    }
    fn input(m: i128, dp: u32) -> Self {
        decimal_text(m, dp)
            .parse()
            .expect("invalid BigDecimal input")
    }
    #[inline(always)]
    fn eval<const OP: u8, const MATCHED: bool>(a: &Self, b: &Self, s: &Settings) -> Self {
        let value = match OP {
            0 => a + b,
            1 => a - b,
            2 => a * b,
            3 => a / b,
            _ => unreachable!(),
        };
        if MATCHED {
            value.with_scale_round(s.dp as i64, bigdecimal::RoundingMode::Down)
        } else {
            value
        }
    }
    fn extract(&self, dp: u32) -> i128 {
        decimal_mantissa(&self.to_string(), dp)
    }
}
macro_rules! fixed_number {
    ($ty:ty, $name:literal, $intermediate:literal) => {
        impl Number for $ty {
            const VARIANT: &'static str = $name;
            const EXACT: bool = false;
            const INTERMEDIATE: &'static str = $intermediate;
            fn representation_dp(_: u32) -> Option<u32> {
                None
            }
            fn input(m: i128, dp: u32) -> Self {
                decimal_text(m, dp).parse().expect("invalid fixed input")
            }
            #[inline(always)]
            fn eval<const OP: u8, const MATCHED: bool>(a: &Self, b: &Self, s: &Settings) -> Self {
                let value = if MATCHED {
                    match OP {
                        0 => a.checked_add(*b),
                        1 => a.checked_sub(*b),
                        2 => a.checked_mul(*b),
                        3 => a.checked_div(*b),
                        _ => unreachable!(),
                    }
                    .expect("fixed arithmetic failed")
                } else {
                    match OP {
                        0 => *a + *b,
                        1 => *a - *b,
                        2 => *a * *b,
                        3 => *a / *b,
                        _ => unreachable!(),
                    }
                };
                if MATCHED {
                    let wide = I64F64::from_num(value);
                    let integral = wide
                        .checked_mul(s.fixed_factor)
                        .expect("fixed decimal scaling overflow")
                        .round_to_zero();
                    Self::from_num(
                        integral
                            .checked_div(s.fixed_factor)
                            .expect("fixed decimal unscaling failed"),
                    )
                } else {
                    value
                }
            }
            fn extract(&self, dp: u32) -> i128 {
                decimal_mantissa(&self.to_string(), dp)
            }
            fn scaled_value(&self, dp: u32) -> f64 {
                self.to_num::<f64>() * 10_f64.powi(dp as i32)
            }
        }
    };
}
fixed_number!(
    I32F32,
    "fixed_I32F32",
    "matched decimal quantization widens to I64F64, then returns I32F32"
);
fixed_number!(
    I64F64,
    "fixed_I64F64",
    "matched decimal quantization uses I64F64"
);
impl Number for f64 {
    const VARIANT: &'static str = "f64";
    const EXACT: bool = false;
    const NATIVE_API: &'static str =
        "IEEE754 operators; matched adds finite assertion and truncation";
    fn representation_dp(_: u32) -> Option<u32> {
        None
    }
    fn input(m: i128, dp: u32) -> Self {
        decimal_text(m, dp).parse().expect("invalid f64 input")
    }
    #[inline(always)]
    fn eval<const OP: u8, const MATCHED: bool>(a: &Self, b: &Self, s: &Settings) -> Self {
        let value = match OP {
            0 => a + b,
            1 => a - b,
            2 => a * b,
            3 => a / b,
            _ => unreachable!(),
        };
        if MATCHED {
            assert!(value.is_finite(), "nonfinite f64 result");
            (value * s.float_factor).trunc() / s.float_factor
        } else {
            value
        }
    }
    fn extract(&self, dp: u32) -> i128 {
        assert!(self.is_finite(), "nonfinite f64 result");
        (*self * 10_f64.powi(dp as i32)).trunc() as i128
    }
    fn scaled_value(&self, dp: u32) -> f64 {
        *self * 10_f64.powi(dp as i32)
    }
}
struct Identity(i128);
impl Number for Identity {
    const VARIANT: &'static str = "baseline_identity_i128";
    const BASELINE: bool = true;
    const TARGET_VISIBILITY: &'static str = "irrelevant";
    const NATIVE_API: &'static str = "identity of a; b is black_boxed, no arithmetic";
    fn input(m: i128, _: u32) -> Self {
        Self(m)
    }
    #[inline(always)]
    fn eval<const OP: u8, const MATCHED: bool>(a: &Self, _: &Self, _: &Settings) -> Self {
        Self(a.0)
    }
    fn extract(&self, _: u32) -> i128 {
        self.0
    }
}
#[derive(Serialize)]
struct CellMeta {
    id: String,
    variant: &'static str,
    workload: String,
    dp: u32,
    mode: &'static str,
    op: &'static str,
    bytes: usize,
    exact_decimal: bool,
    baseline: bool,
    native_api: &'static str,
    intermediate: &'static str,
    representation_dp: Option<u32>,
    output_semantics: &'static str,
    validation_extraction: &'static str,
    target_visibility: &'static str,
}
struct Cell {
    meta: CellMeta,
    timed: Box<dyn Fn(u64) -> u128>,
    validate: Box<dyn Fn() -> Validation>,
}
#[derive(Serialize)]
struct Mismatch {
    pair: usize,
    a: String,
    b: String,
    expected: String,
    actual: String,
    result: String,
}
#[derive(Serialize)]
struct Validation {
    checked_pairs: usize,
    mismatches: usize,
    #[serde(rename = "max_error_units")]
    max_abs_error_target_units: f64,
    examples: Vec<Mismatch>,
}
fn validate<T: Number, const OP: u8, const MATCHED: bool>(
    pairs: &[(T, T)],
    cases: &[Case],
    dp: u32,
) -> Validation {
    let settings = Settings::new(black_box(dp as i32));
    let mut summary = Validation {
        checked_pairs: 0,
        mismatches: 0,
        max_abs_error_target_units: 0.0,
        examples: vec![],
    };
    for (i, ((a, b), c)) in pairs.iter().zip(cases).enumerate() {
        let result = T::eval::<OP, MATCHED>(black_box(a), black_box(b), &settings);
        let actual = result.extract(dp);
        let expected = if T::BASELINE {
            c.a
        } else if !MATCHED && T::NATIVE_ROUNDS {
            c.round[OP as usize]
        } else {
            c.trunc[OP as usize]
        };
        summary.checked_pairs += 1;
        summary.max_abs_error_target_units = summary
            .max_abs_error_target_units
            .max((result.scaled_value(dp) - expected as f64).abs());
        if actual != expected {
            summary.mismatches += 1;
            if summary.examples.len() < 3 {
                summary.examples.push(Mismatch {
                    pair: i,
                    a: c.a.to_string(),
                    b: c.b.to_string(),
                    expected: expected.to_string(),
                    actual: actual.to_string(),
                    result: decimal_text(actual, dp),
                });
            }
        }
    }
    summary
}
#[inline(never)]
fn timed<T: Number, const OP: u8, const MATCHED: bool>(
    pairs: &[(T, T)],
    batches: u64,
    dp: u32,
) -> u128 {
    // OOB scale is opaque once per invocation and read by the arithmetic at runtime.
    let settings = Settings::new(black_box(dp as i32));
    let start = Instant::now();
    for _ in 0..batches {
        for (a, b) in pairs {
            black_box(T::eval::<OP, MATCHED>(
                black_box(a),
                black_box(b),
                &settings,
            ));
        }
    }
    start.elapsed().as_nanos()
}
fn cell<T: Number, const OP: u8, const MATCHED: bool>(
    group: &Group,
    pairs: Arc<Vec<(T, T)>>,
) -> Cell {
    let mode = if MATCHED { "matched" } else { "native" };
    let dp = group.dp;
    let cases = Arc::clone(&group.cases);
    let validation_pairs = Arc::clone(&pairs);
    Cell {
        meta: CellMeta {
            id: format!(
                "{}/{}/dp{}/{}/{}",
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
            baseline: T::BASELINE,
            native_api: T::NATIVE_API,
            intermediate: T::INTERMEDIATE,
            representation_dp: T::representation_dp(dp),
            target_visibility: T::TARGET_VISIBILITY,
            output_semantics: if T::BASELINE {
                "identity baseline"
            } else if MATCHED {
                "truncate toward zero to target dp inside timer"
            } else {
                "native arithmetic result; extraction to target dp outside timer"
            },
            validation_extraction: if T::BASELINE {
                "identity"
            } else if !MATCHED && T::NATIVE_ROUNDS {
                "half away from zero oracle"
            } else {
                "truncate toward zero oracle; binary/f64 numerical differences allowed"
            },
        },
        timed: Box::new(move |batches| timed::<T, OP, MATCHED>(&pairs, batches, dp)),
        validate: Box::new(move || validate::<T, OP, MATCHED>(&validation_pairs, &cases, dp)),
    }
}
fn register<T: Number>(group: &Group, cells: &mut Vec<Cell>) {
    if T::UNSIGNED && group.workload != "positive" {
        return;
    }
    let pairs = Arc::new(
        group
            .cases
            .iter()
            .map(|c| (T::input(c.a, group.dp), T::input(c.b, group.dp)))
            .collect(),
    );
    macro_rules! add {
        ($op:literal) => {
            cells.push(cell::<T, $op, false>(group, Arc::clone(&pairs)));
            cells.push(cell::<T, $op, true>(group, Arc::clone(&pairs)));
        };
    }
    add!(0);
    add!(1);
    add!(2);
    add!(3);
}
fn register_const<const DP: i32>(group: &Group, cells: &mut Vec<Cell>) {
    macro_rules! add {
        ($lib:ident) => {
            register::<$lib::ConstScaleFpdec<i64, DP>>(group, cells);
            register::<$lib::ConstScaleFpdec<u64, DP>>(group, cells);
            register::<$lib::ConstScaleFpdec<i128, DP>>(group, cells);
            register::<$lib::ConstScaleFpdec<u128, DP>>(group, cells);
        };
    }
    add!(current);
    add!(upstream);
}
fn register_matched<T: Number>(group: &Group, cells: &mut Vec<Cell>) {
    let pairs = Arc::new(
        group
            .cases
            .iter()
            .map(|c| (T::input(c.a, group.dp), T::input(c.b, group.dp)))
            .collect(),
    );
    macro_rules! add {
        ($op:literal) => {
            let mut cell = cell::<T, $op, true>(group, Arc::clone(&pairs));
            cell.meta.id.push_str("/known-target");
            cells.push(cell);
        };
    }
    add!(0);
    add!(1);
    add!(2);
    add!(3);
}
fn register_known<const DP: u32>(group: &Group, cells: &mut Vec<Cell>) {
    register_matched::<KnownPrecision<fixdec::D64, DP>>(group, cells);
    register_matched::<KnownPrecision<fixdec::D96, DP>>(group, cells);
    register_matched::<KnownPrecision<rust_decimal::Decimal, DP>>(group, cells);
    register_matched::<KnownPrecision<fastnum::D128, DP>>(group, cells);
    register_matched::<KnownPrecision<fastnum::D256, DP>>(group, cells);
    register_matched::<KnownPrecision<BigDecimal, DP>>(group, cells);
    register_matched::<KnownPrecision<I32F32, DP>>(group, cells);
    register_matched::<KnownPrecision<I64F64, DP>>(group, cells);
    register_matched::<KnownPrecision<f64, DP>>(group, cells);
}
fn register_const_controls<const DP: i32>(group: &Group, cells: &mut Vec<Cell>) {
    register_matched::<current::ConstScaleFpdec<i64, DP>>(group, cells);
    register_matched::<current::ConstScaleFpdec<i128, DP>>(group, cells);
    register_matched::<upstream::ConstScaleFpdec<i64, DP>>(group, cells);
    register_matched::<upstream::ConstScaleFpdec<i128, DP>>(group, cells);
}
fn known_cells(groups: &[Group]) -> Vec<Cell> {
    let mut result = vec![];
    for group in groups {
        match group.dp {
            6 => {
                register_known::<6>(group, &mut result);
                register_const_controls::<6>(group, &mut result);
            }
            7 => {
                register_known::<7>(group, &mut result);
                register_const_controls::<7>(group, &mut result);
            }
            8 => {
                register_known::<8>(group, &mut result);
                register_const_controls::<8>(group, &mut result);
            }
            _ => unreachable!(),
        }
        register_matched::<current::OobScaleFpdec<i64>>(group, &mut result);
        register_matched::<current::OobScaleFpdec<i128>>(group, &mut result);
        register_matched::<upstream::OobScaleFpdec<i64>>(group, &mut result);
        register_matched::<upstream::OobScaleFpdec<i128>>(group, &mut result);
        register_matched::<Identity>(group, &mut result);
    }
    let ids: HashSet<_> = result.iter().map(|c| c.meta.id.as_str()).collect();
    assert_eq!(ids.len(), result.len(), "duplicate known-target cell id");
    result
}
fn cells(groups: &[Group]) -> Vec<Cell> {
    let mut result = vec![];
    for group in groups {
        match group.dp {
            6 => register_const::<6>(group, &mut result),
            7 => register_const::<7>(group, &mut result),
            8 => register_const::<8>(group, &mut result),
            _ => unreachable!(),
        }
        macro_rules! add_oob {
            ($lib:ident) => {
                register::<$lib::OobScaleFpdec<i64>>(group, &mut result);
                register::<$lib::OobScaleFpdec<u64>>(group, &mut result);
                register::<$lib::OobScaleFpdec<i128>>(group, &mut result);
                register::<$lib::OobScaleFpdec<u128>>(group, &mut result);
            };
        }
        add_oob!(current);
        add_oob!(upstream);
        register::<fixdec::D64>(group, &mut result);
        register::<fixdec::D96>(group, &mut result);
        register::<rust_decimal::Decimal>(group, &mut result);
        register::<fastnum::D128>(group, &mut result);
        register::<fastnum::D256>(group, &mut result);
        register::<BigDecimal>(group, &mut result);
        register::<I32F32>(group, &mut result);
        register::<I64F64>(group, &mut result);
        register::<f64>(group, &mut result);
        register::<Identity>(group, &mut result);
    }
    let ids: HashSet<_> = result.iter().map(|c| c.meta.id.as_str()).collect();
    assert_eq!(ids.len(), result.len(), "duplicate cell id");
    result
}
fn emit(meta: &CellMeta, fields: serde_json::Value) {
    let mut row = serde_json::to_value(meta).expect("cannot serialize cell");
    row.as_object_mut().unwrap().extend(
        fields
            .as_object()
            .expect("row fields must be an object")
            .clone(),
    );
    println!("{row}");
}
fn bench(cells: &mut [Cell], round: usize, target_ms: f64) {
    assert!(
        target_ms.is_finite() && target_ms > 0.0,
        "TARGET_MS must be positive and finite"
    );
    let target_ns = (target_ms * 1_000_000.0).ceil() as u128;
    let rotate = round.wrapping_mul(131) % cells.len();
    cells.rotate_left(rotate);
    if round % 2 == 1 {
        cells.reverse();
    }
    for (order, cell) in cells.iter().enumerate() {
        (cell.timed)(1);
        let calibration_ns = (cell.timed)(1).max(1);
        let mut batches = u64::try_from(target_ns.div_ceil(calibration_ns))
            .expect("target duration too large")
            .max(1);
        let mut elapsed = (cell.timed)(batches);
        while elapsed < target_ns {
            batches = batches.checked_mul(2).expect("iteration count overflow");
            elapsed = (cell.timed)(batches);
        }
        let iterations = batches
            .checked_mul(PAIRS as u64)
            .expect("iteration count overflow");
        emit(
            &cell.meta,
            json!({"round": round, "order": order, "wall_ns": elapsed, "iterations": iterations, "ns_per_op": elapsed as f64 / iterations as f64, "target_ms": target_ms, "warmup_pairs": PAIRS, "calibration_pairs": PAIRS, "calibration_ns": calibration_ns}),
        );
    }
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert!(
        args.len() >= 3,
        "usage: decimal-comparison (list[-known]|validate[-known]) FIXTURES | bench[-known] FIXTURES ROUND TARGET_MS"
    );
    let groups = load(&args[2]);
    let mut cells = if matches!(
        args[1].as_str(),
        "list-known" | "validate-known" | "bench-known"
    ) {
        known_cells(&groups)
    } else {
        cells(&groups)
    };
    match args[1].as_str() {
        "list" | "list-known" => {
            assert_eq!(args.len(), 3, "list takes FIXTURES only");
            println!(
                "{}",
                serde_json::to_string(&cells.iter().map(|c| &c.meta).collect::<Vec<_>>()).unwrap()
            );
        }
        "validate" | "validate-known" => {
            assert_eq!(args.len(), 3, "validate takes FIXTURES only");
            let mut failures = 0;
            for cell in cells {
                let validation = (cell.validate)();
                failures += usize::from(cell.meta.exact_decimal && validation.mismatches != 0);
                emit(&cell.meta, serde_json::to_value(validation).unwrap());
            }
            assert_eq!(
                failures, 0,
                "exact-decimal cells disagree with fixture oracle"
            );
        }
        "bench" | "bench-known" => {
            assert_eq!(args.len(), 5, "bench takes FIXTURES ROUND TARGET_MS");
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
    fn assert_known_matches_runtime<T: Number + PartialEq + std::fmt::Debug, const DP: u32>() {
        let (a, b) = (T::input(-12_345_678, DP), T::input(234_567, DP));
        let (known_a, known_b) = (
            KnownPrecision::<T, DP>::input(-12_345_678, DP),
            KnownPrecision::<T, DP>::input(234_567, DP),
        );
        let runtime = Settings::new(DP as i32);
        let ignored = Settings::new(1);
        macro_rules! check {
            ($op:literal) => {
                assert_eq!(
                    KnownPrecision::<T, DP>::eval::<$op, true>(&known_a, &known_b, &ignored).0,
                    T::eval::<$op, true>(&a, &b, &runtime)
                );
            };
        }
        check!(0);
        check!(1);
        check!(2);
        check!(3);
    }
    #[test]
    fn known_target_forwards_values_and_ignores_runtime_settings() {
        assert_known_matches_runtime::<rust_decimal::Decimal, 6>();
        assert_known_matches_runtime::<rust_decimal::Decimal, 7>();
        assert_known_matches_runtime::<rust_decimal::Decimal, 8>();
        assert_known_matches_runtime::<f64, 6>();
        assert_known_matches_runtime::<f64, 7>();
        assert_known_matches_runtime::<f64, 8>();
        assert!(!<KnownPrecision<f64, 6> as Number>::EXACT);
        assert!(!<KnownPrecision<I64F64, 6> as Number>::EXACT);
        assert_eq!(size_of::<KnownPrecision<I64F64, 6>>(), size_of::<I64F64>());
    }
    #[test]
    fn known_target_registry_is_matched_and_disjoint() {
        let groups: Vec<_> = (6..=8)
            .flat_map(|dp| {
                ["positive", "mixed_sign"]
                    .into_iter()
                    .map(move |workload| Group {
                        dp,
                        workload: workload.into(),
                        cases: Arc::new(vec![Case {
                            a: 12_345_678,
                            b: 234_567,
                            trunc: [0; 4],
                            round: [0; 4],
                        }]),
                    })
            })
            .collect();
        let known = known_cells(&groups);
        assert_eq!(known.len(), 432);
        assert!(known
            .iter()
            .all(|c| c.meta.mode == "matched" && c.meta.id.ends_with("/known-target")));
        assert_eq!(
            known
                .iter()
                .filter(|c| c.meta.target_visibility == "runtime")
                .count(),
            96
        );
        assert_eq!(known.iter().filter(|c| !c.meta.exact_decimal).count(), 72);
        let original = cells(&groups);
        let original_ids: HashSet<_> = original.iter().map(|c| c.meta.id.as_str()).collect();
        assert!(known
            .iter()
            .all(|c| !original_ids.contains(c.meta.id.as_str())));
    }
    #[test]
    fn decimal_text_and_extraction_keep_negative_subunit_inputs() {
        assert_eq!(decimal_text(-1, 6), "-0.000001");
        assert_eq!(decimal_mantissa("-1.23456789", 6), -1_234_567);
        assert_eq!(decimal_mantissa("1.234567e2", 6), 123_456_700);
    }
    #[test]
    fn dispatch_has_unique_ids_and_omits_unsigned_mixed_sign() {
        let group = Group {
            dp: 6,
            workload: "mixed_sign".into(),
            cases: Arc::new(vec![Case {
                a: -12_345_678,
                b: 234_567,
                trunc: [0; 4],
                round: [0; 4],
            }]),
        };
        let result = cells(&[group]);
        assert_eq!(result.len(), 18 * 8);
        assert!(result
            .iter()
            .all(|c| !c.meta.variant.ends_with("_u64") && !c.meta.variant.ends_with("_u128")));
        assert_eq!(
            result
                .iter()
                .map(|c| (c.meta.variant, c.meta.op, c.meta.mode))
                .collect::<HashSet<_>>()
                .len(),
            result.len()
        );
    }
    #[test]
    fn static_ops_and_matched_rounding_are_dispatched() {
        type D = current::ConstScaleFpdec<i64, 6>;
        let (a, b, s) = (
            D::input(-12_345_678, 6),
            D::input(234_567, 6),
            Settings::new(6),
        );
        assert_eq!(D::eval::<0, true>(&a, &b, &s).mantissa(), -12_111_111);
        assert_eq!(D::eval::<1, true>(&a, &b, &s).mantissa(), -12_580_245);
        assert_eq!(D::eval::<2, true>(&a, &b, &s).mantissa(), -2_895_888);
        assert_eq!(D::eval::<3, true>(&a, &b, &s).mantissa(), -52_631_776);
    }
    #[test]
    #[should_panic(expected = "unsupported fixture schema")]
    fn fixture_schema_is_checked() {
        fixture_groups(Fixture {
            schema: 2,
            seed: 1,
            description: "bad".into(),
            cases: vec![],
        });
    }
    #[test]
    #[should_panic(expected = "positive fixture would underflow")]
    fn fixture_signs_are_checked_before_unsigned_conversion() {
        fixture_groups(Fixture {
            schema: 1,
            seed: 1,
            description: "bad".into(),
            cases: vec![FixtureCase {
                dp: 6,
                workload: "positive".into(),
                a: "1000000".into(),
                b: "2000000".into(),
                trunc: std::array::from_fn(|_| "0".into()),
                round: std::array::from_fn(|_| "0".into()),
            }],
        });
    }
}
