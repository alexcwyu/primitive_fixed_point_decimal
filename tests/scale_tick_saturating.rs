//! The APIs added for trading code -- `checked_rescale`, `checked_round_to_multiple`,
//! `saturating_add` / `saturating_sub` and `checked_neg` -- against the arbitrary-precision
//! oracle, for all ten inner types and all five roundings.
mod common;
use common::*;

use num_bigint::BigInt;
use num_traits::{Signed, Zero};
use primitive_fixed_point_decimal::{ConstScaleFpdec, OobScaleFpdec as D, Rounding};

/// `value * 10^-diff`, rounded when `diff > 0`; `None` if it does not fit `T`.
fn spec_rescale<T: Int>(a: T, diff: i64, r: Rounding) -> Option<T> {
    let v = a.big();
    let m = if diff > 0 {
        // |a| < 10^39, so past 10^400 the quotient is the same as at 10^400
        round_div(&v, &pow10(diff.min(400) as u32), r)
    } else if v.is_zero() {
        v
    } else if -diff > 400 {
        return None;
    } else {
        v * pow10((-diff) as u32)
    };
    T::from_big(&m)
}

fn spec_round_to_multiple<T: Int>(a: T, step: T, r: Rounding) -> Option<T> {
    if step == T::ZERO {
        return None;
    }
    let s = step.big().abs();
    T::from_big(&(round_div(&a.big(), &s, r) * s))
}

fn run_type<T: Int>() {
    let mut f = Failures::default();
    let name = T::NAME;
    let mut rng = Rng(0x7EC0 ^ T::BITS as u64 ^ ((T::SIGNED as u64) << 40));
    let mut vals = interesting::<T>();
    vals.extend((0..400).map(|_| rng.int::<T>()));
    // The cross products below are large; a stride keeps them to a few hundred per axis.
    let stride = (vals.len() / 300).max(1);

    let d = T::DIGITS as i64;
    let mut diffs: Vec<i64> = (-(d + 2)..=(d + 2)).collect();
    diffs.extend([i32::MIN as i64, i32::MAX as i64, 400, -400]);

    for &a in vals.iter().step_by(stride) {
        let x = D::from_mantissa(a);

        // rescale
        for r in ROUNDINGS {
            for &diff in &diffs {
                let diff32 = diff as i32;
                f.check(|| format!("{name} checked_rescale_ext diff={diff} {r:?}"), || format!("{a:?}"),
                    guarded(|| x.checked_rescale_ext(diff32, r).map(|v| v.mantissa())), spec_rescale(a, diff, r));
            }
        }
        for &diff in &diffs {
            f.check(|| format!("{name} checked_rescale diff={diff}"), || format!("{a:?}"),
                guarded(|| x.checked_rescale(diff as i32).map(|v| v.mantissa())), spec_rescale(a, diff, Rounding::Round));
        }

        // saturating add / sub, checked neg
        for &b in vals.iter().step_by(stride) {
            let y = D::from_mantissa(b);
            let clamp = |n: BigInt| T::from_big(&n).unwrap_or(if n.is_negative() { T::MIN } else { T::MAX });
            f.check(|| format!("{name} saturating_add"), || format!("{a:?} + {b:?}"),
                guarded(|| x.saturating_add(y).mantissa()), clamp(a.big() + b.big()));
            f.check(|| format!("{name} saturating_sub"), || format!("{a:?} - {b:?}"),
                guarded(|| x.saturating_sub(y).mantissa()), clamp(a.big() - b.big()));

            // round to a multiple
            for r in ROUNDINGS {
                f.check(|| format!("{name} checked_round_to_multiple_ext {r:?}"), || format!("{a:?} to {b:?}"),
                    guarded(|| x.checked_round_to_multiple_ext(y, r).map(|v| v.mantissa())), spec_round_to_multiple(a, b, r));
            }
            f.check(|| format!("{name} checked_round_to_multiple"), || format!("{a:?} to {b:?}"),
                guarded(|| x.checked_round_to_multiple(y).map(|v| v.mantissa())), spec_round_to_multiple(a, b, Rounding::Round));
        }
    }
    f.finish(&format!("scale/tick/saturating {name}"));
}

macro_rules! per_type {
    ($($name:ident: $t:ty),*) => { $( #[test] fn $name() { run_type::<$t>(); } )* };
}
per_type!(api_i8: i8, api_i16: i16, api_i32: i32, api_i64: i64, api_i128: i128,
          api_u8: u8, api_u16: u16, api_u32: u32, api_u64: u64, api_u128: u128);

/// `checked_neg` exists only for signed types, so it is checked on its own.
#[test]
fn checked_neg_fails_exactly_at_min() {
    fn run<T: Int + num_traits::Signed>() {
        for a in interesting::<T>() {
            let want = T::from_big(&-a.big());
            let got = D::from_mantissa(a).checked_neg().map(|v| v.mantissa());
            assert_eq!(got, want, "{} {a:?}", T::NAME);
            assert_eq!(ConstScaleFpdec::<T, 3>::from_mantissa(a).checked_neg().map(|v| v.mantissa()), want);
        }
    }
    run::<i8>();
    run::<i16>();
    run::<i32>();
    run::<i64>();
    run::<i128>();
}

#[test]
fn const_scale_rescale_and_tick_documented_cases() {
    type P8 = ConstScaleFpdec<i64, 8>;
    type P2 = ConstScaleFpdec<i64, 2>;
    let p = P8::from_mantissa(12_3456_0000);
    assert_eq!(p.checked_rescale::<2>(), Some(P2::from_mantissa(12_35)));
    assert_eq!(p.checked_rescale_ext::<2>(Rounding::Floor), Some(P2::from_mantissa(12_34)));
    assert_eq!(p.checked_rescale::<8>(), Some(p)); // same scale
    assert_eq!(P8::MAX.checked_rescale::<9>(), None);
    assert_eq!(P8::ZERO.checked_rescale::<{ i32::MAX }>().map(|v| v.mantissa()), Some(0));

    let tick = P2::from_mantissa(5); // 0.05
    let bid = P2::from_mantissa(10_12); // 10.12
    assert_eq!(bid.checked_round_to_multiple(tick), Some(P2::from_mantissa(10_10)));
    assert_eq!(bid.checked_round_to_multiple_ext(tick, Rounding::Ceiling), Some(P2::from_mantissa(10_15)));
    assert_eq!(bid.checked_round_to_multiple_ext(-tick, Rounding::Floor), Some(P2::from_mantissa(10_10)));
    assert_eq!(bid.checked_round_to_multiple(P2::ZERO), None);
}

/// Signed target scales must participate in the difference with their sign,
/// including differences that require more than 32 bits.
#[test]
fn const_rescale_covers_negative_targets_and_extreme_differences() {
    fn check<T: Int, const S: i32, const SR: i32>() {
        let all = interesting::<T>();
        let stride = (all.len() / 40).max(1);
        for a in all.into_iter().step_by(stride).chain([T::MIN, T::MAX, T::ZERO, T::ONE]) {
            for r in ROUNDINGS {
                let actual = ConstScaleFpdec::<T, S>::from_mantissa(a).checked_rescale_ext::<SR>(r).map(|v| v.mantissa());
                assert_eq!(actual, spec_rescale(a, S as i64 - SR as i64, r), "{} {a:?} {S}->{SR} {r:?}", T::NAME);
            }
        }
    }
    fn run<T: Int>() {
        check::<T, 2, -2>(); check::<T, -2, -3>(); check::<T, -3, -2>();
        check::<T, -2, 2>(); check::<T, -2, -2>();
        check::<T, { i32::MAX }, { i32::MIN }>();
        check::<T, { i32::MIN }, { i32::MAX }>();
    }
    run::<i8>(); run::<i16>(); run::<i32>(); run::<i64>(); run::<i128>();
    run::<u8>(); run::<u16>(); run::<u32>(); run::<u64>(); run::<u128>();
}
