//! Tiny cross-revision reproductions kept separate from timed benchmark code.
use std::panic::{catch_unwind, AssertUnwindSafe};

fn observed<T: std::fmt::Debug>(f: impl FnOnce() -> T) -> String {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => format!("{value:?}"),
        Err(_) => "panic".into(),
    }
}

fn main() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut rows = Vec::new();
    macro_rules! probe {
        ($name:expr, $expected:expr, $current:expr, $upstream:expr) => {{
            let got = observed(|| $current);
            let original = observed(|| $upstream);
            assert_eq!(got, $expected, "current regression: {}", $name);
            rows.push(serde_json::json!({"probe":$name,"expected":$expected,
                "current":got,"upstream":original}));
        }};
    }
    macro_rules! exponent_zero {
        ($t:ty) => {
            probe!(
                concat!(stringify!($t), " exponent zero"),
                "Some(21)",
                <$t as current::FpdecInner>::calc_mul_div_exp(3, 7, 0, current::Rounding::Round),
                <$t as upstream::FpdecInner>::calc_mul_div_exp(3, 7, 0, upstream::Rounding::Round)
            );
        };
    }
    exponent_zero!(i64);
    exponent_zero!(u64);
    exponent_zero!(i128);
    exponent_zero!(u128);
    probe!(
        "u64 compensation overflow, diff19",
        "Some(1)",
        <u64 as current::FpdecInner>::calc_mul_div_exp(
            u32::MAX as u64,
            i32::MAX as u64,
            19,
            current::Rounding::Ceiling
        ),
        <u64 as upstream::FpdecInner>::calc_mul_div_exp(
            u32::MAX as u64,
            i32::MAX as u64,
            19,
            upstream::Rounding::Ceiling
        )
    );
    probe!(
        "u128 divider precondition, diff38",
        "Some(1)",
        <u128 as current::FpdecInner>::calc_mul_div_exp(
            8_900_000_000_000_000_000,
            8_900_000_000_000_000_000,
            38,
            current::Rounding::Ceiling
        ),
        <u128 as upstream::FpdecInner>::calc_mul_div_exp(
            8_900_000_000_000_000_000,
            8_900_000_000_000_000_000,
            38,
            upstream::Rounding::Ceiling
        )
    );
    probe!(
        "i128 divider precondition, diff38",
        "Some(1)",
        <i128 as current::FpdecInner>::calc_mul_div_exp(
            8_900_000_000_000_000_000,
            8_900_000_000_000_000_000,
            38,
            current::Rounding::Ceiling
        ),
        <i128 as upstream::FpdecInner>::calc_mul_div_exp(
            8_900_000_000_000_000_000,
            8_900_000_000_000_000_000,
            38,
            upstream::Rounding::Ceiling
        )
    );
    let a = current::ConstScaleFpdec::<i64, 8>::from_mantissa(1);
    let b = current::ConstScaleFpdec::<u64, 8>::from_mantissa(10_000_000_000_000_000_000);
    let ua = upstream::ConstScaleFpdec::<i64, 8>::from_mantissa(1);
    let ub = upstream::ConstScaleFpdec::<u64, 8>::from_mantissa(10_000_000_000_000_000_000);
    probe!(
        "mixed signed/unsigned const mul8",
        "Some(100000000000)",
        a.checked_mul::<u64, 8, 8>(b).map(|v| v.mantissa()),
        ua.checked_mul::<u64, 8, 8>(ub).map(|v| v.mantissa())
    );
    probe!(
        "mixed signed/unsigned const div8",
        "Some(0)",
        a.checked_div::<u64, 8, 8>(b).map(|v| v.mantissa()),
        ua.checked_div::<u64, 8, 8>(ub).map(|v| v.mantissa())
    );
    let a = current::OobScaleFpdec::<i64>::from_mantissa(1);
    let b = current::OobScaleFpdec::<u64>::from_mantissa(10_000_000_000_000_000_000);
    let ua = upstream::OobScaleFpdec::<i64>::from_mantissa(1);
    let ub = upstream::OobScaleFpdec::<u64>::from_mantissa(10_000_000_000_000_000_000);
    probe!(
        "mixed signed/unsigned OOB mul8",
        "Some(100000000000)",
        a.checked_mul(b, 8).map(|v| v.mantissa()),
        ua.checked_mul(ub, 8).map(|v| v.mantissa())
    );
    probe!(
        "mixed signed/unsigned OOB div8",
        "Some(0)",
        a.checked_div(b, -8).map(|v| v.mantissa()),
        ua.checked_div(ub, -8).map(|v| v.mantissa())
    );
    let x = fixdec::D64::try_with_scale(100_000_001, 8).unwrap();
    let half = fixdec::D64::try_with_scale(5, 1).unwrap();
    assert_eq!((x * half).to_raw(), 50_000_000);
    rows.push(
        serde_json::json!({"probe":"D64 native truncation, semantic difference",
        "exact":"0.500000005","native8_raw":(x*half).to_raw(),"half_away8_raw":50_000_001}),
    );
    let a = fixdec::D64::try_with_scale(237, 2).unwrap();
    let b = fixdec::D64::try_with_scale(217, 2).unwrap();
    assert_eq!((a / b).to_raw(), 109_216_589);
    assert_eq!(
        a.checked_div_rounded(b, 8, fixdec::RoundingStrategy::MidpointAwayFromZero)
            .unwrap()
            .to_raw(),
        109_216_590
    );
    rows.push(
        serde_json::json!({"probe":"Existing fixdec example unequal rounding",
        "native8_raw":(a/b).to_raw(),"nearest8_raw":109_216_590}),
    );
    std::panic::set_hook(hook);
    println!("{}", serde_json::to_string_pretty(&rows).unwrap());
}
