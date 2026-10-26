mod common;

use common::{spec_div, spec_mul, spec_ratio, spec_rounding_div, ROUNDINGS};
use primitive_fixed_point_decimal::{ConstScaleFpdec, FpdecInner, OobScaleFpdec, Rounding};

#[test]
fn opaque_oob_multiplication_matches_oracle_across_dispatch_and_scale_bounds() {
    macro_rules! check {
        ($t:ty) => {
            for (a, b) in [(<$t>::MAX, 2), (<$t>::MIN, 3), (5, 2), (0, <$t>::MAX)] {
                for diff in [
                    i32::MIN,
                    -39,
                    -13,
                    -12,
                    -11,
                    -1,
                    0,
                    1,
                    11,
                    12,
                    13,
                    38,
                    39,
                    i32::MAX,
                ] {
                    for r in ROUNDINGS {
                        let got = OobScaleFpdec::from_mantissa(a)
                            .checked_mul_ext(
                                OobScaleFpdec::from_mantissa(b),
                                std::hint::black_box(diff),
                                r,
                            )
                            .map(|x| x.mantissa());
                        assert_eq!(
                            got,
                            spec_mul(a, b, diff, r),
                            "{} {a} * {b}, scale {diff}, {r:?}",
                            stringify!($t)
                        );
                    }
                }
            }
        };
    }
    check!(i32);
    check!(i64);
    check!(i128);
    check!(u128);
}

#[test]
fn twelve_dp_multiplication_preserves_intermediate_overflow_fallbacks() {
    let scale = 10_u128.pow(12);
    let add_limit = u128::MAX - (scale - 1);
    for r in ROUNDINGS {
        for (a, b) in [
            (add_limit - 1, 1),
            (add_limit, 1),
            (add_limit + 1, 1),
            (u128::MAX, 1),
            (u128::MAX, 2),
            (u128::MAX, u128::MAX),
            (0, u128::MAX),
            (u64::MAX as u128, u64::MAX as u128),
            ((1_u128 << 64) + 1, u64::MAX as u128),
            ((1_u128 << 64) + 2, u64::MAX as u128),
        ] {
            assert_eq!(
                a.calc_mul_div_exp(b, 12, r),
                spec_mul(a, b, 12, r),
                "u128 {a} * {b}, {r:?}"
            );
        }
        for (a, b) in [
            (i128::MAX, 1),
            (i128::MAX, 2),
            (i128::MAX, 3),
            (-i128::MAX, 2),
            (-i128::MAX, 3),
            (i128::MIN, 1),
            (i128::MIN, 2),
            (i128::MIN, 3),
            (i128::MIN, -3),
            (i128::MIN, i128::MIN),
            (500_000_000_000, 1),
            (-500_000_000_000, 1),
            (10_i128.pow(19) + 500_000_000_000, 1),
            (-(10_i128.pow(19) + 500_000_000_000), 1),
            (10_i128.pow(20) + 500_000_000_000, 1),
            (-(10_i128.pow(20) + 500_000_000_000), 1),
            ((1_i128 << 64) + 1, u64::MAX as i128),
            ((1_i128 << 64) + 2, u64::MAX as i128),
            (-((1_i128 << 64) + 1), u64::MAX as i128),
            (-((1_i128 << 64) + 2), u64::MAX as i128),
            (0, i128::MAX),
        ] {
            let want = spec_mul(a, b, 12, r);
            assert_eq!(a.calc_mul_div_exp(b, 12, r), want, "inner {a} * {b}, {r:?}");
            assert_eq!(
                ConstScaleFpdec::<i128, 12>::from_mantissa(a)
                    .checked_mul_ext::<i128, 12, 12>(ConstScaleFpdec::from_mantissa(b), r)
                    .map(|x| x.mantissa()),
                want,
                "Const {a} * {b}, {r:?}"
            );
            assert_eq!(
                OobScaleFpdec::from_mantissa(a)
                    .checked_mul_ext(OobScaleFpdec::from_mantissa(b), 12, r)
                    .map(|x| x.mantissa()),
                want,
                "OOB {a} * {b}, {r:?}"
            );
        }
    }
}

#[test]
fn scaled_multiplication_preserves_word_edges_and_asymmetric_fallbacks() {
    let limit = i64::MAX as u128;
    let unsigned = [
        0,
        1,
        3,
        5,
        limit - 1,
        limit,
        limit + 1,
        u64::MAX as u128,
        1 << 64,
        (1 << 126) - 1,
        1 << 127,
        u128::MAX,
    ];
    let signed = [
        i128::MIN,
        -(1 << 126),
        -(limit as i128) - 1,
        -(limit as i128),
        -5,
        -3,
        -1,
        0,
        1,
        3,
        5,
        limit as i128,
        limit as i128 + 1,
        u64::MAX as i128,
        i128::MAX,
    ];
    for i in [0, 1, 8, 10, 12, 18, 37, 38, 39] {
        for r in ROUNDINGS {
            for a in unsigned {
                for b in unsigned {
                    assert_eq!(
                        a.calc_mul_div_exp(b, i, r),
                        spec_mul(a, b, i as i32, r),
                        "u128 {a} * {b} / 10^{i}, {r:?}"
                    );
                }
            }
            for a in signed {
                for b in signed {
                    assert_eq!(
                        a.calc_mul_div_exp(b, i, r),
                        spec_mul(a, b, i as i32, r),
                        "i128 {a} * {b} / 10^{i}, {r:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn rounding_div_preserves_sign_and_rounding_at_word_boundaries() {
    let i64_max = i64::MAX as i128;
    let u64_max = u64::MAX as i128;
    for (a, b) in [
        (5, 2),
        (i64_max, 2),
        (i64_max + 1, 3),
        (i64_max + 2, i64_max + 1),
        (u64_max - 1, 3),
        (u64_max, 2),
        (u64_max + 1, u64_max),
        (u64_max + 2, 3),
    ] {
        for sa in [-1, 1] {
            for sb in [-1, 1] {
                let (a, b) = (a * sa, b * sb);
                for r in ROUNDINGS {
                    assert_eq!(
                        a.rounding_div(b, r),
                        spec_rounding_div(a, b, r),
                        "{a} / {b}, {r:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn rounding_div_handles_signed_min_and_zero_quotients() {
    use Rounding::*;

    for r in ROUNDINGS {
        assert_eq!(i128::MIN.rounding_div(-1, r), None, "{r:?}");
        assert_eq!(i128::MIN.rounding_div(1, r), Some(i128::MIN), "{r:?}");
        assert_eq!(i128::MIN.rounding_div(i128::MIN, r), Some(1), "{r:?}");
        assert_eq!(0i128.rounding_div(i128::MIN, r), Some(0), "{r:?}");
        assert_eq!(1i128.rounding_div(0, r), None, "{r:?}");
        assert_eq!(0i128.rounding_div(0, r), None, "{r:?}");
    }

    // A zero truncated quotient still has the sign of the exact fraction.
    for (r, positive_numerator, negative_numerator) in [
        (Round, 0, 0),
        (Floor, -1, 0),
        (Ceiling, 0, 1),
        (TowardsZero, 0, 0),
        (AwayFromZero, -1, 1),
    ] {
        assert_eq!(1i128.rounding_div(i128::MIN, r), Some(positive_numerator));
        assert_eq!(
            (-1i128).rounding_div(i128::MIN, r),
            Some(negative_numerator)
        );
    }
}

#[test]
fn mul_div_retains_wide_products_and_signed_result_limits() {
    let scale = 10_i128.pow(12);
    let limit = (u128::MAX / scale as u128) as i128;
    for (a, b, c) in [
        (i128::MAX, 2, 2),
        (i128::MIN, 2, 2),
        (i128::MIN, -1, 1),
        (i128::MIN, -1, -1),
        (1i128 << 126, 2, 1),
        (-(1i128 << 126), 2, 1),
        (i128::MAX, i128::MAX, i128::MAX),
        (i128::MIN, i128::MAX, i128::MAX),
        (0, i128::MIN, 0),
    ] {
        for r in ROUNDINGS {
            assert_eq!(
                a.calc_mul_div(b, c, r),
                spec_ratio(a, b, c, r),
                "{a} * {b} / {c}, {r:?}"
            );
        }
    }

    // These products overflow i128, but every rounded quotient fits.
    for (a, b, c) in [
        ((1i128 << 126) + 1, 3, 2),
        ((1i128 << 64) + 1, (1i128 << 64) - 1, (1i128 << 64) + 3),
        ((1i128 << 64) + 2, (1i128 << 64) - 1, (1i128 << 64) + 3),
        ((1i128 << 100) + 1, (1i128 << 64) + 1, (1i128 << 100) + 7),
        (limit, scale, scale),
        (limit + 1, scale, scale),
    ] {
        for sa in [-1, 1] {
            for sb in [-1, 1] {
                for sc in [-1, 1] {
                    let (a, b, c) = (a * sa, b * sb, c * sc);
                    for r in ROUNDINGS {
                        assert_eq!(
                            a.calc_mul_div(b, c, r),
                            spec_ratio(a, b, c, r),
                            "{a} * {b} / {c}, {r:?}"
                        );
                    }
                }
            }
        }
    }
}

fn check_decimal_division<const S: i32>() {
    let p = 10i128.pow(S as u32);
    let mut pairs = vec![
        (0, 0),
        (1, 0),
        (i128::MIN, 1),
        (i128::MIN, p),
        (i128::MIN, -p),
        (i128::MAX, p),
        (i128::MIN, i128::MIN),
        (1, i128::MIN),
        (-1, i128::MIN),
    ];
    for (a, b) in [
        (5, 2 * p),
        (i64::MAX as i128, 3),
        (i64::MAX as i128 + 1, 3),
        (u64::MAX as i128, 3),
        (u64::MAX as i128 + 1, 3),
    ] {
        for sa in [-1, 1] {
            for sb in [-1, 1] {
                pairs.push((a * sa, b * sb));
            }
        }
    }

    for (a, b) in pairs {
        for r in ROUNDINGS {
            let want = spec_div(a, b, -S, r);
            let c = ConstScaleFpdec::<i128, S>::from_mantissa(a)
                .checked_div_ext::<i128, S, S>(ConstScaleFpdec::from_mantissa(b), r)
                .map(|x| x.mantissa());
            let o = OobScaleFpdec::from_mantissa(a)
                .checked_div_ext(OobScaleFpdec::from_mantissa(b), -S, r)
                .map(|x| x.mantissa());
            assert_eq!(c, want, "Const {S}dp: {a} / {b}, {r:?}");
            assert_eq!(o, want, "OOB {S}dp: {a} / {b}, {r:?}");
        }
    }
}

#[test]
fn decimal_division_preserves_all_roundings_at_8_10_12_18dp() {
    check_decimal_division::<8>();
    check_decimal_division::<10>();
    check_decimal_division::<12>();
    check_decimal_division::<18>();
}
