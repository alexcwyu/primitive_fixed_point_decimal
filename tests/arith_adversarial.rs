//! Boundary-steered differential tests for the 128-bit and mixed-width arithmetic fast paths.
//!
//! The oracle is `BigInt` arithmetic written from the definition of each rounding mode. The
//! operands are steered at the thresholds where an implementation switches path: 2^63, 2^64,
//! 2^126, 2^127, 2^128, powers of ten, results exactly at / one past the type's limits, and exact
//! half-way ties. Scale the sampling with `PFPD_ADV_SCALE=n`, vary it with `PFPD_ADV_SEED=n`.
use std::hint::black_box;

use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};
use primitive_fixed_point_decimal::{ConstScaleFpdec as C, FpdecInner, OobScaleFpdec as D, Rounding};

const ROUNDINGS: [Rounding; 5] = [
    Rounding::Round,
    Rounding::Floor,
    Rounding::Ceiling,
    Rounding::TowardsZero,
    Rounding::AwayFromZero,
];

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn u128(&mut self) -> u128 {
        (self.next() as u128) << 64 | self.next() as u128
    }
}

fn big(v: impl Into<BigInt>) -> BigInt {
    v.into()
}
fn pow10(n: u32) -> BigInt {
    BigInt::from(10u8).pow(n)
}
fn pow2(n: u32) -> BigInt {
    BigInt::from(1u8) << n
}

/// `n / d` rounded per `r`, from the definition. `d != 0`.
fn rdiv(n: &BigInt, d: &BigInt, r: Rounding) -> BigInt {
    let q = n / d; // truncates toward zero
    let rem = n - &q * d;
    if rem.is_zero() {
        return q;
    }
    let positive = n.is_negative() == d.is_negative();
    let up: BigInt = &q + 1; // toward +inf when positive
    let down: BigInt = &q - 1; // toward -inf when negative
    let (floor, ceil) = if positive { (q.clone(), up.clone()) } else { (down.clone(), q.clone()) };
    match r {
        Rounding::Floor => floor,
        Rounding::Ceiling => ceil,
        Rounding::TowardsZero => q,
        Rounding::AwayFromZero => if positive { up } else { down },
        Rounding::Round => {
            let twice = rem.abs() * 2;
            let away = if positive { up } else { down };
            if twice >= d.abs() { away } else { q }
        }
    }
}

/// Integer types under test.
trait Int: FpdecInner + Copy + std::fmt::Debug + PartialEq {
    const BITS: u32;
    const SIGNED: bool;
    fn to_big(self) -> BigInt;
    fn from_big(v: &BigInt) -> Option<Self>;
}
macro_rules! int {
    ($t:ty, $signed:expr) => {
        impl Int for $t {
            const BITS: u32 = <$t>::BITS;
            const SIGNED: bool = $signed;
            fn to_big(self) -> BigInt {
                BigInt::from(self)
            }
            fn from_big(v: &BigInt) -> Option<Self> {
                if v.is_negative() {
                    v.to_i128().and_then(|x| <$t>::try_from(x).ok())
                } else {
                    v.to_u128().and_then(|x| <$t>::try_from(x).ok())
                }
            }
        }
    };
}
int!(i16, true);
int!(i32, true);
int!(i64, true);
int!(i128, true);
int!(u16, false);
int!(u32, false);
int!(u64, false);
int!(u128, false);

fn near(rng: &mut Rng, centre: u128) -> u128 {
    let d = rng.below(5) as u128;
    if rng.below(2) == 0 { centre.wrapping_add(d) } else { centre.wrapping_sub(d) }
}

/// A magnitude below `2^bits` biased toward the boundaries where code paths change.
fn mag(rng: &mut Rng, bits: u32) -> u128 {
    let top = if bits == 128 { u128::MAX } else { (1u128 << bits) - 1 };
    let k = rng.below(bits as u64 + 1).min(127) as u32;
    let p = rng.below(39) as u32;
    let w = rng.below(4) as u128;
    let sh = rng.below(65) as u32;
    let m = match rng.below(14) {
        0 => rng.below(300) as u128,
        1 | 2 => rng.u128() >> (127 - k),
        3 | 4 => near(rng, 1u128 << k),
        5 | 6 => near(rng, 10u128.pow(p)),
        7 => near(rng, top),
        8 => near(rng, top / 2 + 1),
        9 => near(rng, 1 << 63),
        10 => near(rng, 1 << 64),
        11 => near(rng, u64::MAX as u128 * w),
        12 => (rng.next() as u128) << sh,
        _ => rng.u128(),
    };
    m & top
}

fn value<T: Int>(rng: &mut Rng) -> T {
    let bits = if T::SIGNED { T::BITS - 1 } else { T::BITS };
    let m = mag(rng, bits);
    let v = if T::SIGNED && rng.below(2) == 0 { -big(m) } else { big(m) };
    // `mag` can reach 2^bits - 1 only, so the lone value signed types miss is MIN
    let v = if T::SIGNED && rng.below(40) == 0 { -pow2(bits) } else { v };
    T::from_big(&v).expect("generated value fits")
}

/// Steer `x` so that `x * y` lands next to `target` (a threshold or tie), if it still fits `T`.
fn steer<T: Int>(rng: &mut Rng, x: &BigInt, target: &BigInt) -> Option<T> {
    if x.is_zero() {
        return None;
    }
    let delta = big(rng.below(5) as i64 - 2);
    let y = target / x + delta;
    T::from_big(&y)
}

fn thresholds<T: Int>(rng: &mut Rng, exp: &BigInt) -> BigInt {
    let max = if T::SIGNED { pow2(T::BITS - 1) } else { pow2(T::BITS) };
    let base = match rng.below(8) {
        0 => max.clone(),
        1 => max - 1,
        2 => max + 1,
        3 => pow2(T::BITS) - 1,
        4 => pow2(126),
        5 => pow2(127),
        6 => pow2(64) * rng.below(8),
        _ => pow2(63) * rng.below(4),
    };
    // times the divisor, offset to straddle every rounding decision (tie = exp / 2)
    let off = match rng.below(6) {
        0 => big(0),
        1 => exp / 2,
        2 => exp / 2 - 1,
        3 => exp / 2 + 1,
        4 => exp - 1,
        _ => big(1),
    };
    base * exp + off
}

fn fits<T: Int>(v: BigInt) -> Option<T> {
    T::from_big(&v)
}

fn spec_mul<T: Int>(a: &BigInt, b: &BigInt, diff: i32, r: Rounding) -> Option<T> {
    fits::<T>(if diff >= 0 { rdiv(&(a * b), &pow10(diff as u32), r) } else { a * b * pow10(diff.unsigned_abs()) })
}

fn spec_div<T: Int>(a: &BigInt, b: &BigInt, diff: i32, r: Rounding) -> Option<T> {
    if b.is_zero() {
        return None;
    }
    fits::<T>(if diff >= 0 {
        rdiv(a, &(b * pow10(diff as u32)), r)
    } else {
        rdiv(&(a * pow10(diff.unsigned_abs())), b, r)
    })
}

struct Stats {
    cases: u64,
    failures: Vec<String>,
}
impl Stats {
    fn check<X: PartialEq + std::fmt::Debug>(&mut self, what: impl FnOnce() -> String, got: X, want: X) {
        self.cases += 1;
        if got != want && self.failures.len() < 20 {
            self.failures.push(format!("{} -> got {got:?}, want {want:?}", what()));
        }
    }
    fn finish(self, name: &str) {
        eprintln!("{name}: {} checks", self.cases);
        assert!(self.failures.is_empty(), "{name}: {} failures, first:\n{}", self.failures.len(), self.failures.join("\n"));
    }
}

fn diffs_for<T: Int>(rng: &mut Rng) -> Vec<i32> {
    let digits = match T::BITS {
        16 => 4,
        32 => 9,
        64 => 18,
        _ => 38,
    };
    let mut v = vec![0, 1, -1, 12.min(digits), -(12.min(digits)), digits, -digits];
    for _ in 0..4 {
        v.push(rng.below(2 * digits as u64 + 1) as i32 - digits);
    }
    v
}

fn run_oob<T: Int>(mut rng: Rng, rounds: u64) {
    let mut st = Stats { cases: 0, failures: vec![] };
    for _ in 0..rounds {
        let (mut a, mut b) = (value::<T>(&mut rng), value::<T>(&mut rng));
        let ds = diffs_for::<T>(&mut rng);
        let diff = ds[rng.below(ds.len() as u64) as usize];
        // steer b so the product / quotient sits on a threshold or a tie
        if rng.below(3) != 0 {
            let exp = pow10(diff.unsigned_abs());
            let tgt = thresholds::<T>(&mut rng, &exp);
            let sign = if T::SIGNED && rng.below(2) == 0 { big(-1) } else { big(1) };
            if rng.below(2) == 0 {
                if let Some(y) = steer::<T>(&mut rng, &a.to_big().abs(), &tgt) {
                    b = if sign.is_negative() && T::SIGNED { T::from_big(&-y.to_big()).unwrap_or(y) } else { y };
                }
            } else if let Some(x) = steer::<T>(&mut rng, &b.to_big().abs(), &tgt) {
                a = x;
            }
        }
        let (ba, bb) = (a.to_big(), b.to_big());
        for &diff in &ds {
            let (x, y) = (D::<T>::from_mantissa(a), D::<T>::from_mantissa(b));
            for r in ROUNDINGS {
                let got = black_box(x).checked_mul_ext(black_box(y), black_box(diff), r).map(|v| v.mantissa());
                st.check(|| format!("{} mul {a:?} {b:?} diff={diff} {r:?}", std::any::type_name::<T>()), got, spec_mul::<T>(&ba, &bb, diff, r));
                let got = black_box(x).checked_div_ext(black_box(y), black_box(diff), r).map(|v| v.mantissa());
                st.check(|| format!("{} div {a:?} {b:?} diff={diff} {r:?}", std::any::type_name::<T>()), got, spec_div::<T>(&ba, &bb, diff, r));
            }
        }
        // rounding_div and calc_mul_div on raw integers, quotient steered to the type limit
        let c = value::<T>(&mut rng);
        for r in ROUNDINGS {
            let want = if c.to_big().is_zero() { None } else { fits::<T>(rdiv(&ba, &c.to_big(), r)) };
            st.check(|| format!("{} rounding_div {a:?} {c:?} {r:?}", std::any::type_name::<T>()), a.rounding_div(c, r), want);
            let want = if c.to_big().is_zero() { None } else { fits::<T>(rdiv(&(&ba * &bb), &c.to_big(), r)) };
            st.check(|| format!("{} calc_mul_div {a:?} {b:?} {c:?} {r:?}", std::any::type_name::<T>()), a.calc_mul_div(b, c, r), want);
        }
    }
    st.finish(std::any::type_name::<T>());
}

/// `calc_mul_div` with `c` derived so the quotient is at a type limit, plus exact half ties.
fn run_mul_div<T: Int>(mut rng: Rng, rounds: u64) {
    let name = std::any::type_name::<T>();
    let mut st = Stats { cases: 0, failures: vec![] };
    let limit = if T::SIGNED { pow2(T::BITS - 1) } else { pow2(T::BITS) };
    for _ in 0..rounds {
        let (a, mut b) = (value::<T>(&mut rng), value::<T>(&mut rng));
        let mut cases: Vec<(T, T, BigInt)> = vec![];
        // c such that a * b / c is just around a limit of the type
        let p = a.to_big() * b.to_big();
        for q in [&limit - 1, limit.clone(), &limit + 1, pow2(T::BITS) - 1, big(1), big(2)] {
            let c = &p / &q;
            cases.extend((-2..=2).map(|d| (a, b, &c + d)));
        }
        // exact tie: a * b / c = (2m + 1) / 2 with b = (2m + 1) * t and c = 2 * a * t
        let t = big(1 + rng.below(1000));
        let odd = big(2 * rng.below(1u64 << 40) + 1);
        if let Some(tie_b) = T::from_big(&(&odd * &t)) {
            b = tie_b;
            cases.push((a, b, a.to_big() * &t * 2));
        }
        for (a, b, c) in cases {
            let Some(c) = T::from_big(&c) else { continue };
            let p = a.to_big() * b.to_big();
            for r in ROUNDINGS {
                let want = if c.to_big().is_zero() { None } else { fits::<T>(rdiv(&p, &c.to_big(), r)) };
                st.check(|| format!("{name} calc_mul_div {a:?} {b:?} {c:?} {r:?}"), a.calc_mul_div(b, c, r), want);
            }
        }
    }
    st.finish(&format!("mul_div {name}"));
}

fn run_mixed<I: Int, J: Int>(mut rng: Rng, rounds: u64) {
    let mut st = Stats { cases: 0, failures: vec![] };
    let digits = match I::BITS {
        16 => 4,
        32 => 9,
        64 => 18,
        _ => 38,
    };
    for _ in 0..rounds {
        let (a, mut b) = (value::<I>(&mut rng), value::<J>(&mut rng));
        let diff = rng.below(2 * digits + 1) as i32 - digits as i32;
        if rng.below(2) == 0 {
            let tgt = thresholds::<I>(&mut rng, &pow10(diff.unsigned_abs()));
            if let Some(y) = steer::<J>(&mut rng, &a.to_big().abs(), &tgt) {
                b = y;
            }
        }
        let (ba, bb) = (a.to_big(), b.to_big());
        for d in [diff, 12.min(digits as i32), -(12.min(digits as i32))] {
            let (x, y) = (D::<I>::from_mantissa(a), D::<J>::from_mantissa(b));
            for r in ROUNDINGS {
                let got = black_box(x).checked_mul_ext(black_box(y), black_box(d), r).map(|v| v.mantissa());
                st.check(|| format!("{}x{} mul {a:?} {b:?} diff={d} {r:?}", std::any::type_name::<I>(), std::any::type_name::<J>()), got, spec_mul::<I>(&ba, &bb, d, r));
                let got = black_box(x).checked_div_ext(black_box(y), black_box(d), r).map(|v| v.mantissa());
                st.check(|| format!("{}x{} div {a:?} {b:?} diff={d} {r:?}", std::any::type_name::<I>(), std::any::type_name::<J>()), got, spec_div::<I>(&ba, &bb, d, r));
            }
        }
    }
    st.finish(&format!("mixed {} x {}", std::any::type_name::<I>(), std::any::type_name::<J>()));
}

/// `ConstScaleFpdec` entry points with constant scales: the compiler sees `diff` and rounding.
macro_rules! const_case {
    ($st:expr, $rng:expr, $rounds:expr, $I:ty, $S:literal, $S2:literal, $SR:literal) => {{
        for _ in 0..$rounds {
            let (mut a, mut b) = (value::<$I>(&mut $rng), value::<$I>(&mut $rng));
            let mdiff: i32 = $S + $S2 - $SR;
            let ddiff: i32 = $S - $S2 - $SR;
            if $rng.below(2) == 0 {
                let tgt = thresholds::<$I>(&mut $rng, &pow10(mdiff.unsigned_abs()));
                if let Some(y) = steer::<$I>(&mut $rng, &a.to_big().abs(), &tgt) {
                    b = y;
                }
            } else {
                let tgt = thresholds::<$I>(&mut $rng, &pow10(ddiff.unsigned_abs()));
                if let Some(x) = steer::<$I>(&mut $rng, &b.to_big().abs(), &tgt) {
                    a = x;
                }
            }
            let (ba, bb) = (a.to_big(), b.to_big());
            let (x, y) = (C::<$I, $S>::from_mantissa(a), C::<$I, $S2>::from_mantissa(b));
            for r in ROUNDINGS {
                let got = x.checked_mul_ext::<$I, $S2, $SR>(y, r).map(|v| v.mantissa());
                $st.check(|| format!("const {} {}/{}/{} mul {a:?} {b:?} {r:?}", stringify!($I), $S, $S2, $SR), got, spec_mul::<$I>(&ba, &bb, mdiff, r));
                let got = x.checked_div_ext::<$I, $S2, $SR>(y, r).map(|v| v.mantissa());
                $st.check(|| format!("const {} {}/{}/{} div {a:?} {b:?} {r:?}", stringify!($I), $S, $S2, $SR), got, spec_div::<$I>(&ba, &bb, ddiff, r));
            }
        }
    }};
}

fn run_const(mut rng: Rng, rounds: u64) {
    let mut st = Stats { cases: 0, failures: vec![] };
    // multiplication diff = S + S2 - SR, division diff = S - S2 - SR
    const_case!(st, rng, rounds, i128, 6, 6, 0); // mul 12
    const_case!(st, rng, rounds, i128, 8, 8, 4); // mul 12
    const_case!(st, rng, rounds, i128, 12, 12, 12); // mul 12, div -12
    const_case!(st, rng, rounds, i128, 0, 0, 12); // div -12
    const_case!(st, rng, rounds, i128, 6, 6, 12); // div -12
    const_case!(st, rng, rounds, i128, 10, 10, 10); // mul 10, div -10
    const_case!(st, rng, rounds, i128, 8, 8, 8); // mul 8, div -8
    const_case!(st, rng, rounds, i128, 18, 18, 18); // mul 18, div -18
    const_case!(st, rng, rounds, i128, 12, 6, 6); // div 0
    const_case!(st, rng, rounds, u128, 6, 6, 0);
    const_case!(st, rng, rounds, u128, 12, 12, 12);
    const_case!(st, rng, rounds, u128, 0, 0, 12);
    const_case!(st, rng, rounds, u128, 18, 18, 18);
    const_case!(st, rng, rounds, i64, 6, 6, 0);
    const_case!(st, rng, rounds, i64, 8, 8, 8);
    const_case!(st, rng, rounds, u64, 8, 8, 8);
    st.finish("const scale");
}

fn rounds() -> u64 {
    env("PFPD_ADV_SCALE", 1) * if cfg!(debug_assertions) { 6_000 } else { 60_000 }
}
fn seed(n: u64) -> Rng {
    Rng(env("PFPD_ADV_SEED", 1).wrapping_mul(0x1234_5678_9ABC_DEF1).wrapping_add(n))
}

#[test]
fn oob_i128() { run_oob::<i128>(seed(1), rounds()) }
#[test]
fn oob_u128() { run_oob::<u128>(seed(2), rounds()) }
#[test]
fn oob_i64() { run_oob::<i64>(seed(3), rounds()) }
#[test]
fn oob_u64() { run_oob::<u64>(seed(4), rounds()) }
#[test]
fn oob_i32() { run_oob::<i32>(seed(5), rounds()) }
#[test]
fn oob_u32() { run_oob::<u32>(seed(6), rounds()) }
#[test]
fn oob_i16() { run_oob::<i16>(seed(7), rounds()) }
#[test]
fn oob_u16() { run_oob::<u16>(seed(8), rounds()) }
#[test]
fn mul_div_i128() { run_mul_div::<i128>(seed(9), rounds()) }
#[test]
fn mul_div_u128() { run_mul_div::<u128>(seed(10), rounds()) }
#[test]
fn mixed_i128_u128() { run_mixed::<i128, u128>(seed(11), rounds()) }
#[test]
fn mixed_i128_i64() { run_mixed::<i128, i64>(seed(12), rounds()) }
#[test]
fn mixed_u128_u64() { run_mixed::<u128, u64>(seed(13), rounds()) }
#[test]
fn mixed_i64_i128() { run_mixed::<i64, i128>(seed(14), rounds()) }
#[test]
fn mixed_u64_u128() { run_mixed::<u64, u128>(seed(15), rounds()) }
#[test]
fn mixed_i32_i128() { run_mixed::<i32, i128>(seed(16), rounds()) }
#[test]
fn mixed_u128_i64() { run_mixed::<u128, i64>(seed(17), rounds()) }
#[test]
fn mixed_i128_i16() { run_mixed::<i128, i16>(seed(18), rounds()) }
#[test]
fn const_scale() { run_const(seed(19), rounds() / 4) }

/// Integer-operand entry points: `rounding_div`, `calc_mul_div`, round-to-multiple and rem.
fn run_int_ops<T: Int>(mut rng: Rng, rounds: u64) {
    let name = std::any::type_name::<T>();
    let mut st = Stats { cases: 0, failures: vec![] };
    for _ in 0..rounds {
        let (a, mut n, c) = (value::<T>(&mut rng), value::<T>(&mut rng), value::<T>(&mut rng));
        if rng.below(3) == 0 {
            // a divisor around a / k so the quotient is a small integer or sits on a tie
            let k = 1 + rng.below(9);
            if let Some(y) = steer::<T>(&mut rng, &big(1), &(a.to_big() / k)) {
                n = y;
            }
        }
        let (ba, bn, bc) = (a.to_big(), n.to_big(), c.to_big());
        let x = D::<T>::from_mantissa(a);
        for r in ROUNDINGS {
            let want = if bn.is_zero() { None } else { fits::<T>(rdiv(&ba, &bn, r)) };
            st.check(|| format!("{name} div_int {a:?} {n:?} {r:?}"), x.checked_div_int_ext(n, r).map(|v| v.mantissa()), want);
            let want = if bc.is_zero() { None } else { fits::<T>(rdiv(&(&ba * &bn), &bc, r)) };
            st.check(|| format!("{name} mul_ratio {a:?} {n:?} {c:?} {r:?}"), x.checked_mul_ratio_ext(n, c, r).map(|v| v.mantissa()), want);
            // nearest / floor / ceiling multiple of |step|; the sign of step must not matter
            let want = if bn.is_zero() { None } else { fits::<T>(rdiv(&ba, &bn.abs(), r) * bn.abs()) };
            let got = x.checked_round_to_multiple_ext(D::<T>::from_mantissa(n), r).map(|v| v.mantissa());
            st.check(|| format!("{name} round_to_multiple {a:?} {n:?} {r:?}"), got, want);
        }
        let want = if bn.is_zero() { None } else { fits::<T>(&ba % &bn) };
        st.check(|| format!("{name} rem {a:?} {n:?}"), C::<T, 3>::from_mantissa(a).checked_rem(C::<T, 3>::from_mantissa(n)).map(|v| v.mantissa()), want);
        st.check(|| format!("{name} add {a:?} {n:?}"), x.checked_add(D::<T>::from_mantissa(n)).map(|v| v.mantissa()), fits::<T>(&ba + &bn));
        st.check(|| format!("{name} sub {a:?} {n:?}"), x.checked_sub(D::<T>::from_mantissa(n)).map(|v| v.mantissa()), fits::<T>(&ba - &bn));
        // rounding away digits, including the exact digit counts where 10^d stops fitting
        for d in [1, 2, 4, 9, 12, 18, 19, 38, 39] {
            let d = d as i32;
            for r in ROUNDINGS {
                let e = pow10(d as u32);
                let want = fits::<T>(rdiv(&ba, &e, r) * &e);
                let got = x.checked_round_diff_ext(d, r).map(|v| v.mantissa());
                st.check(|| format!("{name} round_diff_ext {a:?} d={d} {r:?}"), got, want);
            }
        }
    }
    st.finish(&format!("int ops {name}"));
}

#[test]
fn int_ops_i128() { run_int_ops::<i128>(seed(21), rounds()) }
#[test]
fn int_ops_u128() { run_int_ops::<u128>(seed(22), rounds()) }
#[test]
fn int_ops_i64() { run_int_ops::<i64>(seed(23), rounds()) }
#[test]
fn int_ops_u64() { run_int_ops::<u64>(seed(24), rounds()) }
#[test]
fn int_ops_i32() { run_int_ops::<i32>(seed(25), rounds()) }
#[test]
fn int_ops_u32() { run_int_ops::<u32>(seed(26), rounds()) }
#[test]
fn int_ops_i16() { run_int_ops::<i16>(seed(27), rounds()) }
#[test]
fn int_ops_u16() { run_int_ops::<u16>(seed(28), rounds()) }

#[test]
fn neg_and_abs_reject_only_min() {
    macro_rules! t { ($($t:ty),*) => {$(
        for v in [<$t>::MIN, <$t>::MIN + 1, -1, 0, 1, <$t>::MAX] {
            let x = D::<$t>::from_mantissa(v);
            assert_eq!(x.checked_neg().map(|r| r.mantissa()), v.checked_neg(), "neg {v}");
            assert_eq!(x.checked_abs().map(|r| r.mantissa()), v.checked_abs(), "abs {v}");
        }
    )*}}
    t!(i16, i32, i64, i128);
}
