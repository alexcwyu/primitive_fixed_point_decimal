//! Differential tests: every arithmetic entry point against an arbitrary-
//! precision oracle (`tests/common`), for ALL ten inner integer types, ALL five
//! rounding modes, signs, and the magnitudes where the fast paths hand over to
//! the slow ones.
//!
//! Why this exists: the in-crate tests compare `calc_mul_div` with
//! `calc_mul_div_exp` -- two implementations by the same author that share
//! operand magnitudes (`10^i - k` against `u128::MAX - j*113`), so a flaw in the
//! boundary of the "happy path" guard (`leading_zeros(a) + leading_zeros(b) >= N`)
//! is invisible to them. The oracle shares nothing with the implementation.
//!
//! Scale the sampling up with `PFPD_ORACLE_SCALE=10 cargo test --release --test oracle`.
mod common;
use common::*;

use num_bigint::BigInt;
use num_traits::{Signed, Zero};
use primitive_fixed_point_decimal::{ConstScaleFpdec, FpdecInner, OobScaleFpdec as D, ParseError, Rounding};

fn scale() -> usize {
    std::env::var("PFPD_ORACLE_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1)
}

// ---------------------------------------------------------------------------
// The checks
// ---------------------------------------------------------------------------

/// The `diff` values for rounding: `diffs()` plus the first few beyond the type's
/// digits (where `10^diff` stops fitting and the library has a separate branch) and
/// one far beyond.
fn round_diffs<T: Int>() -> Vec<i32> {
    let d = T::DIGITS as i32;
    let mut v = diffs::<T>();
    v.extend([d + 2, d + 3, d + 40]);
    v.sort();
    v.dedup();
    v
}

fn check_round<T: Int>(f: &mut Failures, a: T, ds: &[i32]) {
    let x = D::from_mantissa(a);
    let name = T::NAME;
    for r in ROUNDINGS {
        for &d in ds {
            let want = spec_round(a, d, r);
            f.check(|| format!("{name} checked_round_diff_ext diff={d} {r:?}"), || format!("{a:?}"),
                guarded(|| x.checked_round_diff_ext(d, r).map(|v| v.mantissa())), want);
            // the panicking twin must agree: same value, or a panic exactly when `None`
            f.check(|| format!("{name} round_diff_ext diff={d} {r:?}"), || format!("{a:?}"),
                Ok(guarded(|| x.round_diff_ext(d, r).mantissa()).ok()), want);
        }
        let want = spec_round_huge(a, r);
        f.check(|| format!("{name} checked_round_diff_ext i32::MAX {r:?}"), || format!("{a:?}"),
            guarded(|| x.checked_round_diff_ext(i32::MAX, r).map(|v| v.mantissa())), want);
        // diff = i32::MIN: nothing to drop (this used to overflow `-diff_scale`)
        f.check(|| format!("{name} checked_round_diff_ext i32::MIN {r:?}"), || format!("{a:?}"),
            guarded(|| x.checked_round_diff_ext(i32::MIN, r).map(|v| v.mantissa())), Some(a));
    }
    // `round_diff` is `round_diff_ext(.., Rounding::Round)`
    for &d in ds {
        f.check(|| format!("{name} checked_round_diff diff={d}"), || format!("{a:?}"),
            guarded(|| x.checked_round_diff(d).map(|v| v.mantissa())), spec_round(a, d, Rounding::Round));
    }
}

fn check_pair<T: Int>(f: &mut Failures, a: T, b: T, ds: &[i32]) {
    let (x, y) = (D::from_mantissa(a), D::from_mantissa(b));
    let name = T::NAME;

    // add / sub: exact, `None` exactly when out of range.
    let sum = T::from_big(&(a.big() + b.big()));
    let dif = T::from_big(&(a.big() - b.big()));
    f.check(|| format!("{name} checked_add"), || format!("{a:?} + {b:?}"),
        guarded(|| x.checked_add(y).map(|v| v.mantissa())), sum);
    f.check(|| format!("{name} checked_sub"), || format!("{a:?} - {b:?}"),
        guarded(|| x.checked_sub(y).map(|v| v.mantissa())), dif);
    f.check(|| format!("{name} checked_mul_int"), || format!("{a:?} * {b:?}"),
        guarded(|| x.checked_mul_int(b).map(|v| v.mantissa())), T::from_big(&(a.big() * b.big())));

    for r in ROUNDINGS {
        f.check(|| format!("{name} rounding_div {r:?}"), || format!("{a:?} / {b:?}"),
            guarded(|| a.rounding_div(b, r)), spec_rounding_div(a, b, r));
        f.check(|| format!("{name} checked_div_int_ext {r:?}"), || format!("{a:?} / {b:?}"),
            guarded(|| x.checked_div_int_ext(b, r).map(|v| v.mantissa())), spec_rounding_div(a, b, r));
    }

    for &d in ds {
        // With diff <= 0 the multiply has no rounding step, so one mode is enough.
        let mul_modes: &[Rounding] = if d > 0 { &ROUNDINGS } else { &ROUNDINGS[..1] };
        for &r in mul_modes {
            f.check(|| format!("{name} mul diff={d} {r:?}"), || format!("{a:?} * {b:?}"),
                guarded(|| x.checked_mul_ext(y, d, r).map(|v| v.mantissa())), spec_mul(a, b, d, r));
        }
        for r in ROUNDINGS {
            f.check(|| format!("{name} div diff={d} {r:?}"), || format!("{a:?} / {b:?}"),
                guarded(|| x.checked_div_ext(y, d, r).map(|v| v.mantissa())), spec_div(a, b, d, r));
        }
    }
}

fn check_ratio<T: Int>(f: &mut Failures, a: T, n: T, d: T) {
    let x = D::from_mantissa(a);
    let name = T::NAME;
    for r in ROUNDINGS {
        f.check(|| format!("{name} mul_ratio {r:?}"), || format!("{a:?} * {n:?} / {d:?}"),
            guarded(|| x.checked_mul_ratio_ext(n, d, r).map(|v| v.mantissa())), spec_ratio(a, n, d, r));
    }
}

fn run_type<T: Int>() {
    let mut f = Failures::default();
    let k = scale();
    let ds = diffs::<T>();
    let all = interesting::<T>();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ T::BITS as u64 ^ ((T::SIGNED as u64) << 40));

    // 8-bit types: EXHAUSTIVE. Every pair, every diff, every rounding is a proof.
    if T::BITS == 8 {
        let lo = T::min_big();
        let hi = T::max_big();
        let mut vals = Vec::new();
        let mut v = lo.clone();
        while v <= hi {
            vals.push(T::from_big(&v).unwrap());
            v += 1;
        }
        let rds = round_diffs::<T>();
        for &a in &vals {
            check_round(&mut f, a, &rds);
            for &b in &vals {
                check_pair(&mut f, a, b, &ds);
            }
        }
        for &a in vals.iter().step_by(7) {
            for &n in vals.iter().step_by(5) {
                for &d in vals.iter().step_by(3) {
                    check_ratio(&mut f, a, n, d);
                }
            }
        }
        f.finish(&format!("oracle {} (exhaustive)", T::NAME));
        return;
    }

    // 1. A deterministic grid over the boundary values.
    let stride = (all.len() / (60 * k)).max(1);
    let core: Vec<T> = all.iter().step_by(stride).copied().collect();
    for &a in &core {
        for &b in &core {
            check_pair(&mut f, a, b, &ds);
        }
    }
    // 2. Every boundary value against a rotating sample of the others.
    for (i, &a) in all.iter().enumerate() {
        for j in 0..(8 * k) {
            let b = all[(i * 7 + j * 13 + 1) % all.len()];
            check_pair(&mut f, a, b, &ds);
        }
    }
    // 3. Products that land on word boundaries; 4. exact half-way ties.
    for (a, b) in product_edge_pairs::<T>(&mut rng, 3 * k) {
        check_pair(&mut f, a, b, &ds);
    }
    for (a, b) in tie_pairs::<T>(&mut rng, 2 * k) {
        check_pair(&mut f, a, b, &ds);
    }
    // 4b. Quotients that land exactly on (or just past) MIN / MAX, and `a * n / n == a`
    // (which makes `a = MIN` give a quotient of exactly MIN through the wide path).
    for (a, b) in exact_limit_pairs::<T>() {
        check_pair(&mut f, a, b, &ds);
    }
    for &a in &all {
        for j in 0..(4 * k) {
            let n = all[(a.big().bits() as usize * 31 + j * 17 + 3) % all.len()];
            check_ratio(&mut f, a, n, n);
            if let Some(neg_n) = T::from_big(&-n.big()) {
                check_ratio(&mut f, a, n, neg_n);
            }
        }
    }
    // 5. Uniform-bit-length random pairs.
    for _ in 0..(3000 * k) {
        let (a, b): (T, T) = (rng.int(), rng.int());
        check_pair(&mut f, a, b, &ds);
    }
    // 6. rounding: every boundary value, plus uniform random magnitudes.
    let rds = round_diffs::<T>();
    for &a in &all {
        check_round(&mut f, a, &rds);
    }
    for _ in 0..(1500 * k) {
        check_round(&mut f, rng.int::<T>(), &rds);
    }
    // 7. a*n/d ratios.
    for _ in 0..(2500 * k) {
        let (a, n, d): (T, T, T) = (rng.int(), rng.int(), rng.int());
        check_ratio(&mut f, a, n, d);
    }
    for _ in 0..(1500 * k) {
        let pick = |rng: &mut Rng| all[rng.below(all.len() as u64) as usize];
        let (a, n, d) = (pick(&mut rng), pick(&mut rng), pick(&mut rng));
        check_ratio(&mut f, a, n, d);
    }
    f.finish(&format!("oracle {}", T::NAME));
}

macro_rules! per_type {
    ($($name:ident: $t:ty),*) => { $( #[test] fn $name() { run_type::<$t>(); } )* };
}
per_type!(oracle_i8: i8, oracle_i16: i16, oracle_i32: i32, oracle_i64: i64, oracle_i128: i128,
          oracle_u8: u8, oracle_u16: u16, oracle_u32: u32, oracle_u64: u64, oracle_u128: u128);

// ---------------------------------------------------------------------------
// The oracle itself must be right, or everything above proves nothing.
// ---------------------------------------------------------------------------

#[test]
fn oracle_round_div_matches_hand_computed_table() {
    use Rounding::*;
    // (n, d, [Round, Floor, Ceiling, TowardsZero, AwayFromZero])
    let table: &[(i64, i64, [i64; 5])] = &[
        (7, 2, [4, 3, 4, 3, 4]),        // 3.5   tie rounds AWAY from zero
        (-7, 2, [-4, -4, -3, -3, -4]),  // -3.5
        (7, -2, [-4, -4, -3, -3, -4]),
        (-7, -2, [4, 3, 4, 3, 4]),
        (5, 3, [2, 1, 2, 1, 2]),        // 1.67
        (-5, 3, [-2, -2, -1, -1, -2]),
        (4, 3, [1, 1, 2, 1, 2]),        // 1.33
        (-4, 3, [-1, -2, -1, -1, -2]),
        (6, 3, [2, 2, 2, 2, 2]),        // exact: every mode agrees
        (-6, 3, [-2, -2, -2, -2, -2]),
        (0, 5, [0, 0, 0, 0, 0]),
        (1, 3, [0, 0, 1, 0, 1]),
        (-1, 3, [0, -1, 0, 0, -1]),
        (1, 2, [1, 0, 1, 0, 1]),        // 0.5
        (-1, 2, [-1, -1, 0, 0, -1]),
    ];
    for (n, d, want) in table {
        for (i, r) in [Round, Floor, Ceiling, TowardsZero, AwayFromZero].into_iter().enumerate() {
            let got = round_div(&BigInt::from(*n), &BigInt::from(*d), r);
            assert_eq!(got, BigInt::from(want[i]), "{n}/{d} {r:?}");
        }
    }
}

#[test]
fn oracle_agrees_with_native_i64_arithmetic() {
    let mut rng = Rng(7);
    for _ in 0..20_000 {
        let a = (rng.next() as i64) >> (rng.below(60) as u32);
        let b = loop {
            let b = (rng.next() as i64) >> (rng.below(60) as u32);
            if b != 0 {
                break b;
            }
        };
        let (q, rem) = (a.wrapping_div(b), a.wrapping_rem(b));
        let want_trunc = q;
        let got = round_div(&BigInt::from(a), &BigInt::from(b), Rounding::TowardsZero);
        assert_eq!(got, BigInt::from(want_trunc));
        // floor via Rust's own `div_euclid`/`rem_euclid` relation
        let floor = if rem != 0 && ((rem < 0) != (b < 0)) { q - 1 } else { q };
        assert_eq!(round_div(&BigInt::from(a), &BigInt::from(b), Rounding::Floor), BigInt::from(floor));
        let _ = BigInt::from(a).abs();
    }
}

/// The oracle's `interesting()` set must actually contain the boundary cases the
/// suite is meant to hit; guard against it silently shrinking.
#[test]
fn boundary_generators_cover_the_edges() {
    let v = interesting::<i128>();
    assert!(v.contains(&i128::MIN) && v.contains(&i128::MAX) && v.contains(&0) && v.contains(&-1));
    assert!(v.contains(&10_i128.pow(38)) && v.contains(&(5 * 10_i128.pow(37))));
    assert!(v.iter().any(|x| *x == (1i128 << 63) + 1) && v.iter().any(|x| *x == (1i128 << 126) - 1));
    let u = interesting::<u64>();
    assert!(u.contains(&u64::MAX) && u.contains(&10_u64.pow(19)) && u.contains(&(5 * 10_u64.pow(18))));
    assert!(v.len() > 1000, "i128 boundary set unexpectedly small: {}", v.len());
}

#[test]
fn product_edge_pairs_really_straddle_word_boundaries() {
    let mut rng = Rng(11);
    let pairs = product_edge_pairs::<i128>(&mut rng, 8);
    let near = |p: &BigInt, k: u32| (p.abs() - (BigInt::from(1u8) << k)).abs() <= BigInt::from(1u64 << 40);
    for k in [62u32, 63, 64, 126, 127] {
        assert!(
            pairs.iter().any(|(a, b)| near(&(a.big() * b.big()), k) && a.big().abs() > BigInt::from(1u8) << 8),
            "no pair with |a*b| near 2^{k}"
        );
    }
    // And ties: a*b mod 10^i == 10^i / 2 exactly.
    let ties = tie_pairs::<i64>(&mut rng, 2);
    for (a, b) in ties.iter().take(200) {
        let p = (a.big() * b.big()).abs();
        assert!((1..=18u32).any(|i| (&p % pow10(i)) == pow10(i - 1) * 5), "{a} * {b} is not a tie");
    }
}

/// Calling the safe public helper directly must also handle EXP[0] = 1. The
/// decimal wrapper bypasses this case, so testing only the wrapper misses UB
/// in the specialised 64/128-bit implementations.
#[test]
fn public_power_helpers_include_zero_exponent() {
    fn run<T: Int>() {
        let mut f = Failures::default();
        let mut vals = vec![T::ZERO, T::ONE, T::TEN, T::MIN, T::MAX];
        if T::SIGNED {
            vals.push(T::ZERO - T::ONE);
        }
        let mut exps: Vec<usize> = (0..=T::DIGITS as usize + 1).collect();
        exps.push(usize::MAX);
        for &a in &vals {
            for &b in &vals {
                for &i in &exps {
                    for r in ROUNDINGS {
                        let valid = i <= T::DIGITS as usize;
                        let mul = if valid { spec_mul(a, b, i as i32, r) } else { None };
                        let div = if valid { spec_div(a, b, i as i32, r) } else { None };
                        f.check(|| format!("{} calc_mul_div_exp i={i} {r:?}", T::NAME), || format!("{a} * {b}"),
                            guarded(|| a.calc_mul_div_exp(b, i, r)), mul);
                        f.check(|| format!("{} calc_div_div_exp i={i} {r:?}", T::NAME), || format!("{a} / {b}"),
                            guarded(|| a.calc_div_div_exp(b, i, r)), div);
                    }
                }
            }
        }
        f.finish(&format!("public power helpers {}", T::NAME));
    }
    run::<i8>(); run::<i16>(); run::<i32>(); run::<i64>(); run::<i128>();
    run::<u8>(); run::<u16>(); run::<u32>(); run::<u64>(); run::<u128>();
}

/// A mixed-sign/width result of exactly MIN must survive the magnitude-to-sign
/// conversion, including the one magnitude that cannot be cast to i128 first.
#[test]
fn mixed_unsigned_rhs_can_produce_exact_signed_min() {
    use primitive_fixed_point_decimal::ConstScaleFpdec as C;
    fn run<T: Int + num_traits::Signed>() {
        let a = T::ZERO - T::ONE;
        let b = 1_u128 << (T::BITS - 1);
        for r in ROUNDINGS {
            assert_eq!(D::from_mantissa(a).checked_mul_ext(D::from_mantissa(b), 0, r).map(|v| v.mantissa()), Some(T::MIN));
            assert_eq!(C::<T, 0>::from_mantissa(a).checked_mul_ext::<_, 0, 0>(C::from_mantissa(b), r).map(|v| v.mantissa()), Some(T::MIN));
        }
        assert_eq!((C::<T, 0>::from_mantissa(a) * C::<u128, 0>::from_mantissa(b)).mantissa(), T::MIN);
    }
    run::<i8>(); run::<i16>(); run::<i32>(); run::<i64>(); run::<i128>();
}

/// `calc_mul_div` of the 128-bit types when `a * b` does not fit: the sign of the quotient is
/// taken from `a ^ b ^ c` (so `c == a ^ b`, which makes that xor exactly 0, is a POSITIVE
/// result), a negative quotient may be exactly `MIN` but not `MIN - 1`, and a high word of the
/// product that reaches the divisor (`mhigh >= c`) is an overflow, not a wrong quotient.
#[test]
fn calc_mul_div_128bit_signs_and_quotient_limits() {
    fn run<T: Int>() {
        let mut f = Failures::default();
        let mut triples: Vec<(T, T, T)> = Vec::new();
        let mut push = |a: &BigInt, b: &BigInt, c: &BigInt| {
            if let (Some(a), Some(b), Some(c)) = (T::from_big(a), T::from_big(b), T::from_big(c)) {
                triples.push((a, b, c));
            }
        };

        // (1) c = a ^ b, with a product that overflows the type
        let vals = interesting::<T>();
        let stride = (vals.len() / 70).max(1);
        let core: Vec<T> = vals.iter().step_by(stride).copied().collect();
        for &a in &core {
            for &b in &core {
                let c = a ^ b;
                push(&a.big(), &b.big(), &c.big());
            }
        }

        // (2) the exact quotient q = a * b / c is on, or just beyond, the limits of the type.
        // `c = 1` matters: then `|a * b|` is the quotient itself, below 2^128 for the 128-bit
        // types, so the high word is 0 and the whole product is handed to the final-division
        // branch (`last_dividend >= 2^127`), where the quotient is odd -- `-(2^127 + 1)` must be
        // `None`, not a wrapped `i128::MAX` (`q > MAX + 1` mutated to `q > MAX + 2` survived).
        let top = if T::SIGNED { BigInt::from(1u8) << (T::BITS - 1) } else { BigInt::from(1u8) << T::BITS };
        for d in -2..=2i32 {
            let magnitude = &top + d;
            for q in [magnitude.clone(), -magnitude] {
                for c in [1, 2, 3, 5, 7, 10, 11, 1000, 65_537, 4_294_967_311u64] {
                    let c = BigInt::from(c);
                    for j in 1..=64u32 {
                        let j = BigInt::from(j);
                        if !(&q % &j == BigInt::from(0)) {
                            continue;
                        }
                        let (a, b) = (&q / &j, &c * &j);
                        // every sign pattern that keeps the quotient
                        push(&a, &b, &c);
                        push(&-&a, &-&b, &c);
                        push(&-&a, &b, &-&c);
                        push(&a, &-&b, &-&c);
                    }
                }
            }
        }

        // (3) the high word of |a| * |b| equal to, or next to, |c|
        let max_mag = if T::SIGNED { BigInt::from(1u8) << (T::BITS - 1) } else { (BigInt::from(1u8) << T::BITS) - 1 };
        let mut cs: Vec<BigInt> = Vec::new();
        for k in [T::BITS - 1, T::BITS - 2, T::BITS - 3] {
            for d in -2..=2i32 {
                cs.push((BigInt::from(1u8) << k) + d);
            }
        }
        cs.push(&max_mag - 1);
        cs.push(max_mag.clone());
        for c in cs.iter().filter(|c| **c > BigInt::from(0)) {
            for a in [&max_mag, &(&max_mag - 1), &(&max_mag - 2), c, &(c + 1), &(c * 2)] {
                if *a <= BigInt::from(0) {
                    continue;
                }
                let b = (c << 128usize).div_ceil_(a);
                for d in -1..=1i32 {
                    let b = &b + d;
                    for (sa, sb, sc) in [(1, 1, 1), (-1, 1, 1), (1, -1, -1), (-1, -1, -1), (-1, -1, 1), (1, 1, -1)] {
                        push(&(a * sa), &(&b * sb), &(c * sc));
                    }
                }
            }
        }

        // (4) `reduce2_big` (divisor >= 2^127): the first normalised dividend equals the divisor
        // EXACTLY. With z leading zeros in the high word of P = |a * b|, that dividend is the top
        // 128 bits of P, so choose c = P >> (128 - z) (and its neighbours). Then the
        // `dividend < divisor` test decides between "subtract now" and "shift one more bit first"
        // (which drops the top bit): `<` mutated to `<=` returned wrong quotients and survived,
        // e.g. (2^90 - 1) * (2^90 + 1) / (2^128 - 1) = 2^52.
        let two128 = BigInt::from(1u8) << 128usize;
        let mut near_pow2: Vec<(BigInt, BigInt)> = Vec::new();
        for k in 40..=128usize {
            for m in [k, k + 1, k.saturating_sub(1)] {
                if m > 128 {
                    continue;
                }
                for (j, jj) in [(1i32, 1i32), (1, 0), (0, 1), (1, -1), (2, 2), (3, 1), (0, 0), (-1, 1)] {
                    let a: BigInt = (BigInt::from(1u8) << k) - j;
                    let b: BigInt = (BigInt::from(1u8) << m) + jj;
                    near_pow2.push((a, b));
                }
            }
        }
        let mut rng = Rng(0x00C0_FFEE ^ T::BITS as u64);
        for _ in 0..400 {
            near_pow2.push((BigInt::from(rng.u128()), BigInt::from(rng.u128())));
        }
        for (a, b) in &near_pow2 {
            let prod = a * b;
            let high = &prod >> 128usize;
            if high.is_zero() || high >= (BigInt::from(1u8) << 127usize) {
                continue; // no overflow, or no leading zero for the normalisation
            }
            let z = 128 - high.bits() as usize;
            let c0: BigInt = &prod >> (128 - z);
            for d in -1..=1i32 {
                let c = &c0 + d;
                if c < two128 {
                    push(a, b, &c);
                    push(&-a, b, &-&c);
                }
            }
        }

        for (a, b, c) in triples {
            let x = D::from_mantissa(a);
            for r in ROUNDINGS {
                let want = spec_ratio(a, b, c, r);
                f.check(|| format!("{} calc_mul_div {r:?}", T::NAME), || format!("{a:?} * {b:?} / {c:?}"),
                    guarded(|| a.calc_mul_div(b, c, r)), want);
                f.check(|| format!("{} checked_mul_ratio_ext {r:?}", T::NAME), || format!("{a:?} * {b:?} / {c:?}"),
                    guarded(|| x.checked_mul_ratio_ext(b, c, r).map(|v| v.mantissa())), want);
            }
        }
        f.finish(&format!("128-bit calc_mul_div limits {}", T::NAME));
    }
    run::<i128>();
    run::<u128>();
    run::<i64>();
    run::<u64>();
}

trait CeilDiv {
    fn div_ceil_(&self, d: &BigInt) -> BigInt;
}
impl CeilDiv for BigInt {
    fn div_ceil_(&self, d: &BigInt) -> BigInt {
        let q = self / d;
        if &q * d < *self { q + 1 } else { q }
    }
}

/// A mixed-width result of exactly `-(MAX)` (`MIN + 1`) must survive the magnitude-to-sign
/// conversion: `-5 * (2 * MAX) / 10` as `T` times a `u128` that does not fit `T`.
#[test]
fn mixed_unsigned_rhs_can_produce_signed_min_plus_one() {
    fn run<T: Int + num_traits::Signed>() {
        let mut f = Failures::default();
        let five = T::fit_small(-5);
        for extra in 0..=3u32 {
            let Some(b) = ToU128::to_u128_(&(T::MAX.big() * 2 + BigInt::from(extra))) else { continue };
            for r in ROUNDINGS {
                for diff in [1, 0, 2] {
                    f.check(|| format!("{} mixed mul {r:?} diff={diff}", T::NAME), || format!("-5 * {b} (u128)"),
                        guarded(|| D::from_mantissa(five).checked_mul_ext(D::from_mantissa(b), diff, r).map(|v| v.mantissa())),
                        spec_mul_mixed(five, b, diff, r));
                }
            }
        }
        f.finish(&format!("mixed MIN+1 {}", T::NAME));
    }
    run::<i8>(); run::<i16>(); run::<i32>(); run::<i64>(); run::<i128>();
}

trait SmallInt: Sized {
    fn fit_small(v: i32) -> Self;
}
impl<T: Int> SmallInt for T {
    fn fit_small(v: i32) -> Self {
        T::from_big(&BigInt::from(v)).unwrap()
    }
}
trait ToU128 {
    fn to_u128_(&self) -> Option<u128>;
}
impl ToU128 for BigInt {
    fn to_u128_(&self) -> Option<u128> {
        num_traits::ToPrimitive::to_u128(self)
    }
}

/// The i32 ratio/division fast path must round the widened product once,
/// including negative divisors, ties and narrowing at either signed limit.
#[test]
fn i32_widened_ratio_rounding_boundaries() {
    let values = [i32::MIN, i32::MIN + 1, -65_535, -32_768, -10_000, -3, -2, -1,
        0, 1, 2, 3, 10_000, 32_768, 65_535, i32::MAX - 1, i32::MAX];
    let mut f = Failures::default();
    for a in values {
        for b in values {
            for c in values {
                for r in ROUNDINGS {
                    let want = spec_ratio(a, b, c, r);
                    f.check(|| format!("i32 widened ratio {r:?}"), || format!("{a} * {b} / {c}"),
                        guarded(|| a.calc_mul_div(b, c, r)), want);
                    f.check(|| format!("i32 widened wrapper ratio {r:?}"), || format!("{a} * {b} / {c}"),
                        guarded(|| D::from_mantissa(a).checked_mul_ratio_ext(b, c, r).map(|x| x.mantissa())), want);
                }
            }
        }
    }
    f.finish("i32 widened ratio boundaries");
}

// ---------------------------------------------------------------------------
// Integer -> decimal conversion (`fpdec!(5)`, `TryFrom<int>`), every source and inner type.
// ---------------------------------------------------------------------------

/// The mantissa of `v` at `scale` from the definition: `v * 10^scale`, or an error --
/// `Precision` when a negative scale would drop non-zero digits, `Overflow` when the
/// mantissa does not fit the inner type. Zero is representable at every scale.
fn spec_from_int<I: Int>(v: &BigInt, scale: i32) -> Result<I, ParseError> {
    if v.is_zero() {
        return Ok(I::from_big(&BigInt::from(0)).unwrap());
    }
    // 10^39 is beyond every type (and `10^i32::MAX` is not a number to compute): a non-zero
    // value is either far too big (scale > 0) or has digits to drop (scale < 0).
    if scale.unsigned_abs() > 39 {
        return Err(if scale > 0 { ParseError::Overflow } else { ParseError::Precision });
    }
    let m = if scale >= 0 {
        v * pow10(scale as u32)
    } else {
        let d = pow10(scale.unsigned_abs());
        if !(v % &d).is_zero() {
            return Err(ParseError::Precision);
        }
        v / d
    };
    I::from_big(&m).ok_or(ParseError::Overflow)
}

fn conv_values<S: Int>() -> Vec<S> {
    let mut set = std::collections::BTreeSet::new();
    let mut add = |b: BigInt| {
        if S::from_big(&b).is_some() {
            set.insert(b);
        }
    };
    for base in [0u32, 1, 2, 5, 9, 10, 11, 99, 100, 101, 127, 128, 129, 255, 256, 257, 999, 1000, 1001, 32767, 32768, 65535, 65536] {
        add(BigInt::from(base));
        add(-BigInt::from(base));
    }
    for k in 0..=S::BITS {
        let p = BigInt::from(1u8) << k;
        for d in -2..=2i32 {
            add(&p + d);
            add(-(&p + d));
        }
    }
    for k in 0..=38u32 {
        for p in [pow10(k), pow10(k) * 5] {
            for d in -1..=1i32 {
                add(&p + d);
                add(-(&p + d));
            }
        }
    }
    add(S::max_big());
    add(S::min_big());
    add(S::max_big() - 1);
    add(S::min_big() + 1);
    set.iter().filter_map(S::from_big).collect()
}

fn conv_oob<I: Int, S: Int>(f: &mut Failures, vals: &[S])
where
    D<I>: TryFrom<(S, i32), Error = ParseError>,
{
    let d = I::DIGITS as i32;
    let mut scales: Vec<i32> = (-12..=12).collect();
    scales.extend([d - 1, d, d + 1, d + 2, -d - 1, -d, -d + 1, 40, -40, i32::MAX, i32::MIN]);
    for &v in vals {
        for &s in &scales {
            f.check(|| format!("{} <- {} oob scale {s}", I::NAME, S::NAME), || format!("{v:?}"),
                guarded(|| D::<I>::try_from((v, s)).map(|x| x.mantissa())), spec_from_int::<I>(&v.big(), s));
        }
    }
}

fn conv_const<I: Int, S: Int, const SC: i32>(f: &mut Failures, vals: &[S])
where
    ConstScaleFpdec<I, SC>: TryFrom<S, Error = ParseError>,
{
    for &v in vals {
        f.check(|| format!("{} <- {} const scale {SC}", I::NAME, S::NAME), || format!("{v:?}"),
            guarded(|| ConstScaleFpdec::<I, SC>::try_from(v).map(|x| x.mantissa())), spec_from_int::<I>(&v.big(), SC));
    }
}

/// The conversion widens to the inner type first for `scale > 0` and scales in the SOURCE type
/// first otherwise, so a source narrower than the inner type behaves differently at scale 1 and 0:
/// `S > 0` mutated to `S > 1` turned `ConstScaleFpdec<i32, 1>::try_from(100_i8)` (1000) into
/// `Overflow`, and the whole suite still passed.
#[test]
fn integer_conversion_matches_spec_for_every_source_and_inner_type() {
    use primitive_fixed_point_decimal::ConstScaleFpdec as C;
    let _ = C::<i8, 0>::ZERO;
    let mut f = Failures::default();
    macro_rules! pair {
        ($i:ty, $s:ty) => {{
            let vals = conv_values::<$s>();
            conv_oob::<$i, $s>(&mut f, &vals);
            conv_const::<$i, $s, -3>(&mut f, &vals);
            conv_const::<$i, $s, -2>(&mut f, &vals);
            conv_const::<$i, $s, -1>(&mut f, &vals);
            conv_const::<$i, $s, 0>(&mut f, &vals);
            conv_const::<$i, $s, 1>(&mut f, &vals);
            conv_const::<$i, $s, 2>(&mut f, &vals);
            conv_const::<$i, $s, 3>(&mut f, &vals);
            conv_const::<$i, $s, 20>(&mut f, &vals);
            conv_const::<$i, $s, -20>(&mut f, &vals);
        }};
    }
    macro_rules! for_inner {
        ($i:ty) => {{
            pair!($i, i8); pair!($i, i16); pair!($i, i32); pair!($i, i64); pair!($i, i128);
            pair!($i, u8); pair!($i, u16); pair!($i, u32); pair!($i, u64); pair!($i, u128);
        }};
    }
    for_inner!(i8); for_inner!(i16); for_inner!(i32); for_inner!(i64); for_inner!(i128);
    for_inner!(u8); for_inner!(u16); for_inner!(u32); for_inner!(u64); for_inner!(u128);
    f.finish("integer conversion");
}

/// Large scale conversion must exercise chunking, not just reject every input.
/// These binary floats have exact scaled values just below a half-integer:
/// changing a chunk size moves that value across the rounding boundary.
#[test]
fn float_conversion_large_scales_round_known_binary_values() {
    use primitive_fixed_point_decimal::ConstScaleFpdec as C;
    // Decode the binary value exactly, independently of floating-point powi.
    fn exact(bits: u64, fraction_bits: u32, bias: i32, scale: u32) -> BigInt {
        let fraction = bits & ((1u64 << fraction_bits) - 1);
        let exponent = (bits >> fraction_bits) as i32;
        let (significand, power) = if exponent == 0 {
            (fraction, 1 - bias - fraction_bits as i32)
        } else {
            (fraction | (1u64 << fraction_bits), exponent - bias - fraction_bits as i32)
        };
        let n = BigInt::from(significand) * pow10(scale);
        if power >= 0 { n << power as usize }
        else { round_div(&n, &(BigInt::from(1u8) << power.unsigned_abs() as usize), Rounding::Round) }
    }
    for (bits, expected) in [(346_542_026u32, 169_446i32), (388_211_038, 5_287_181)] {
        assert_eq!(exact(bits as u64, 23, 127, 31), BigInt::from(expected));
        let f = f32::from_bits(bits);
        for sign in [-1i32, 1] {
            let want = expected * sign;
            assert_eq!(D::<i32>::try_from((f * sign as f32, 31)).map(|x| x.mantissa()), Ok(want));
            assert_eq!(C::<i32, 31>::try_from(f * sign as f32).map(|x| x.mantissa()), Ok(want));
        }
    }
    let bits = 6_091_913_674_683_243u64;
    let expected = 3_009_805_264_091i64;
    assert_eq!(exact(bits, 52, 1023, 320), BigInt::from(expected));
    let f = f64::from_bits(bits);
    for sign in [-1i64, 1] {
        let want = expected * sign;
        assert_eq!(D::<i64>::try_from((f * sign as f64, 320)).map(|x| x.mantissa()), Ok(want));
        assert_eq!(C::<i64, 320>::try_from(f * sign as f64).map(|x| x.mantissa()), Ok(want));
    }
}

/// The powi intrinsic gave different powers of ten at runtime and when folded.
/// Pin the existing debug result in both profiles, for both wrappers and signs.
#[test]
fn float_conversion_large_scale_is_profile_independent() {
    use primitive_fixed_point_decimal::ConstScaleFpdec as C;
    let f = f64::from_bits(4_339_908_218_620_558);
    for sign in [-1i64, 1] {
        let expected = 2_144_199_556_925i64 * sign;
        assert_eq!(D::<i64>::try_from((f * sign as f64, 320)).map(|x| x.mantissa()), Ok(expected));
        assert_eq!(C::<i64, 320>::try_from(f * sign as f64).map(|x| x.mantissa()), Ok(expected));
    }
}

/// Large sums/differences of three const scales must not wrap to an in-range diff.
#[test]
fn const_scale_arithmetic_differences_do_not_wrap_into_range() {
    use primitive_fixed_point_decimal::ConstScaleFpdec as C;
    fn run<T: Int>() {
        let max = C::<T, { i32::MAX }>::from_mantissa(T::ONE);
        let min = C::<T, { i32::MIN }>::from_mantissa(T::ONE);
        for r in ROUNDINGS {
            // MAX + MAX - (-2) = 2^32; MIN + MIN - 0 = -2^32.
            assert_eq!(max.checked_mul_ext::<T, { i32::MAX }, -2>(max, r).map(|x| x.mantissa()), None);
            assert_eq!(min.checked_mul_ext::<T, { i32::MIN }, 0>(min, r).map(|x| x.mantissa()), None);
            // MAX - MIN - (-1) = 2^32; MIN - MAX - 1 = -2^32.
            assert_eq!(max.checked_div_ext::<T, { i32::MIN }, -1>(min, r).map(|x| x.mantissa()), None);
            assert_eq!(min.checked_div_ext::<T, { i32::MAX }, 1>(max, r).map(|x| x.mantissa()), None);
        }
    }
    run::<i8>(); run::<i16>(); run::<i32>(); run::<i64>(); run::<i128>();
    run::<u8>(); run::<u16>(); run::<u32>(); run::<u64>(); run::<u128>();
}
