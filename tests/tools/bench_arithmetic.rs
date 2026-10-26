// Build and compare using python3 tests/tools/bench_arithmetic.py.
// ALT_BUILD selects straightforward correct primitive implementations; otherwise
// benchmark the public decimal APIs on realistic small price/quantity mantissas.
use primitive_fixed_point_decimal::{
    ConstScaleFpdec as C, FpdecInner, OobScaleFpdec as D, Rounding,
};
use std::{hint::black_box, time::Instant};
fn bench<A: Copy, B: Copy, O, F: FnMut(A, B) -> O>(name: &str, data: &[(A, B)], mut f: F) {
    let mut acc = 0;
    for _ in 0..100 {
        for &(a, b) in data {
            black_box(f(a, b));
        }
    }
    let n = 15_000_000;
    let start = Instant::now();
    for i in 0..n {
        let (a, b) = data[i & 1023];
        black_box(f(black_box(a), black_box(b)));
        acc += 1;
    }
    black_box(acc);
    println!(
        "{name:<42} {:.2} ns/op",
        start.elapsed().as_nanos() as f64 / n as f64
    );
}
fn simple<W: FpdecInner>(a: W, b: W) -> Option<W> {
    let q = a.checked_div(&b)?;
    let rem = a % b;
    let (r, d) = (rem.unsigned_abs(), b.unsigned_abs());
    if r >= d - r {
        if (a < W::ZERO) != (b < W::ZERO) {
            q.checked_sub(&W::ONE)
        } else {
            q.checked_add(&W::ONE)
        }
    } else {
        Some(q)
    }
}
fn main() {
    let alt = option_env!("ALT_BUILD").is_some();
    let mut rng = 123456789u64;
    macro_rules! test {
        ($t:ty) => {
            let data: Vec<($t, $t)> = (0..1024)
                .map(|_| {
                    rng ^= rng << 13;
                    rng ^= rng >> 7;
                    rng ^= rng << 17;
                    let a = (rng % 100_000_000) as $t;
                    rng ^= rng << 13;
                    rng ^= rng >> 7;
                    rng ^= rng << 17;
                    (a, (rng % 65535 + 1) as $t)
                })
                .collect();
            bench(concat!(stringify!($t), " add"), &data, |a, b| {
                if alt {
                    a.checked_add(b).unwrap_or(0)
                } else {
                    C::<$t, 4>::from_mantissa(a)
                        .checked_add(C::from_mantissa(b))
                        .map_or(0, |x| x.mantissa())
                }
            });
            bench(concat!(stringify!($t), " sub"), &data, |a, b| {
                if alt {
                    a.checked_sub(b).unwrap_or(0)
                } else {
                    C::<$t, 4>::from_mantissa(a)
                        .checked_sub(C::from_mantissa(b))
                        .map_or(0, |x| x.mantissa())
                }
            });
            bench(concat!(stringify!($t), " mul_int"), &data, |a, b| {
                if alt {
                    a.checked_mul(b).unwrap_or(0)
                } else {
                    C::<$t, 4>::from_mantissa(a)
                        .checked_mul_int(b)
                        .map_or(0, |x| x.mantissa())
                }
            });
            bench(concat!(stringify!($t), " rescale drop"), &data, |a, _| {
                if alt {
                    a.rounding_div(100, Rounding::Round).unwrap()
                } else {
                    C::<$t, 4>::from_mantissa(a)
                        .checked_rescale::<2>()
                        .unwrap()
                        .mantissa()
                }
            });
            bench(concat!(stringify!($t), " rescale grow"), &data, |a, _| {
                if alt {
                    a.checked_mul(100).unwrap_or(0)
                } else {
                    C::<$t, 4>::from_mantissa(a)
                        .checked_rescale::<6>()
                        .map_or(0, |x| x.mantissa())
                }
            });
            bench(
                concat!(stringify!($t), " oob rescale drop"),
                &data,
                |a, _| {
                    if alt {
                        a.rounding_div(100, Rounding::Round).unwrap()
                    } else {
                        D::<$t>::from_mantissa(a)
                            .checked_rescale(2)
                            .unwrap()
                            .mantissa()
                    }
                },
            );
            bench(concat!(stringify!($t), " round multiple"), &data, |a, b| {
                if alt {
                    a.rounding_div(b, Rounding::Round)
                        .and_then(|q| q.checked_mul(b))
                        .unwrap_or(0)
                } else {
                    C::<$t, 4>::from_mantissa(a)
                        .checked_round_to_multiple(C::from_mantissa(b))
                        .map_or(0, |x| x.mantissa())
                }
            });
        };
    }
    test!(i32);
    test!(i64);
    test!(u64);
    test!(i128);
    macro_rules! extra {
        ($t:ty,$w:ty) => {
            let data: Vec<($t, $t)> = (0..1024)
                .map(|_| {
                    rng ^= rng << 13;
                    rng ^= rng >> 7;
                    rng ^= rng << 17;
                    let a = (rng % 100_000_000) as $t;
                    rng ^= rng << 13;
                    rng ^= rng >> 7;
                    rng ^= rng << 17;
                    (a, (rng % 65535 + 1) as $t)
                })
                .collect();
            // Check Options before timing: map_or(0) would hide a false overflow.
            for &(a, b) in &data {
                let x = C::<$t, 4>::from_mantissa(a);
                let y = C::<$t, 4>::from_mantissa(b);
                let narrow = |q| <$t>::try_from(q).ok();
                assert_eq!(x.checked_mul::<_, 4, 4>(y).map(|v| v.mantissa()),
                    simple(a as $w * b as $w, 10000).and_then(narrow));
                assert_eq!(x.checked_div::<_, 4, 4>(y).map(|v| v.mantissa()),
                    simple(a as $w * 10000, b as $w).and_then(narrow));
                assert_eq!(x.checked_div_int(b).map(|v| v.mantissa()),
                    simple(a as $w, b as $w).and_then(narrow));
                assert_eq!(x.checked_mul_ratio(b, 65537 as $t).map(|v| v.mantissa()),
                    simple(a as $w * b as $w, 65537).and_then(narrow));
                assert_eq!(x.checked_round(2).map(|v| v.mantissa()),
                    simple(a as $w, 100).and_then(|q| narrow(q * 100)));
            }
            bench(concat!(stringify!($t), " decimal mul"), &data, |a, b| {
                if alt {
                    simple(a as $w * b as $w, 10000)
                        .and_then(|q| <$t>::try_from(q).ok())
                        .unwrap_or(0)
                } else {
                    C::<$t, 4>::from_mantissa(a)
                        .checked_mul::<_, 4, 4>(C::from_mantissa(b))
                        .map_or(0, |x| x.mantissa())
                }
            });
            bench(concat!(stringify!($t), " decimal div"), &data, |a, b| {
                if alt {
                    simple(a as $w * 10000, b as $w)
                        .and_then(|q| <$t>::try_from(q).ok())
                        .unwrap_or(0)
                } else {
                    C::<$t, 4>::from_mantissa(a)
                        .checked_div::<_, 4, 4>(C::from_mantissa(b))
                        .map_or(0, |x| x.mantissa())
                }
            });
            bench(concat!(stringify!($t), " div_int"), &data, |a, b| {
                if alt {
                    simple(a as $w, b as $w)
                        .and_then(|q| <$t>::try_from(q).ok())
                        .unwrap_or(0)
                } else {
                    C::<$t, 4>::from_mantissa(a)
                        .checked_div_int(b)
                        .map_or(0, |x| x.mantissa())
                }
            });
            bench(concat!(stringify!($t), " mul_ratio"), &data, |a, b| {
                if alt {
                    simple(a as $w * b as $w, 65537)
                        .and_then(|q| <$t>::try_from(q).ok())
                        .unwrap_or(0)
                } else {
                    C::<$t, 4>::from_mantissa(a)
                        .checked_mul_ratio(b, 65537 as $t)
                        .map_or(0, |x| x.mantissa())
                }
            });
            bench(concat!(stringify!($t), " round"), &data, |a, _| {
                if alt {
                    simple(a as $w, 100)
                        .and_then(|q| <$t>::try_from(q * 100).ok())
                        .unwrap_or(0)
                } else {
                    C::<$t, 4>::from_mantissa(a)
                        .checked_round(2)
                        .map_or(0, |x| x.mantissa())
                }
            });
        };
    }
    extra!(i32, i64);
    extra!(i64, i128);
    extra!(u64, u128);
    // These bounded operands fit in 128 bits even before rescaling.
    extra!(i128, i128);
    extra!(u128, u128);
    i32_div_signs(alt);
    i128_round_signs_and_runtime(alt);
}

// The generated mantissas fit i64; multiply the rounded quotient in i128 so
// rounding an i64 limit up does not overflow the reference implementation.
#[inline(always)]
fn round_small(a: i64, exp: i64, mode: Rounding) -> i128 {
    let exp = exp as u64;
    let (q, rem) = (a.unsigned_abs() / exp, a.unsigned_abs() % exp);
    let away = rem != 0 && match mode {
        Rounding::Round => rem >= exp - rem,
        Rounding::Floor => a < 0,
        Rounding::Ceiling => a > 0,
        Rounding::TowardsZero => false,
        Rounding::AwayFromZero => true,
    };
    let rounded = (q as i128 + i128::from(away)) * exp as i128;
    if a < 0 { -rounded } else { rounded }
}

fn i128_round_signs_and_runtime(alt: bool) {
    let modes = [Rounding::Round, Rounding::Floor, Rounding::Ceiling,
        Rounding::TowardsZero, Rounding::AwayFromZero];
    for (a, want) in [(149, [100,100,200,100,200]), (150, [200,100,200,100,200]),
        (-149, [-100,-200,-100,-100,-200]), (-150, [-200,-200,-100,-100,-200]),
        (0, [0;5]), (200, [200;5]), (-200, [-200;5])] {
        for (mode, want) in modes.into_iter().zip(want) {
            assert_eq!(round_small(a, 100, mode), want);
        }
    }
    let mut rng = 0x1234_5678_9abc_def0u64;
    let data: Vec<_> = (0..1024).map(|i| {
        rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
        let a = (rng as i64) >> 40;
        let diff = [0,1,2,4,8][i % 5];
        (a as i128, (diff, 10_i64.pow(diff as u32), modes[(i / 5) % 5]))
    }).collect();
    for &(a, (diff, exp, mode)) in &data {
        assert_eq!(C::<i128,4>::from_mantissa(a).checked_round(2).map(|x| x.mantissa()),
            Some(round_small(a as i64, 100, Rounding::Round)));
        assert_eq!(D::from_mantissa(a).checked_round_diff_ext(diff, mode).map(|x| x.mantissa()),
            Some(round_small(a as i64, exp, mode)));
    }
    bench("i128 round signs", &data, |a, _| if alt {
        Some(round_small(a as i64, 100, Rounding::Round))
    } else {
        C::<i128,4>::from_mantissa(a).checked_round(2).map(|x| x.mantissa())
    });
    bench("i128 round runtime", &data, |a, (diff, exp, mode)| if alt {
        Some(round_small(a as i64, exp, mode))
    } else {
        D::from_mantissa(a).checked_round_diff_ext(diff, mode).map(|x| x.mantissa())
    });
}

// Sign-diverse runtime decimal division, with the same operand barriers on both paths.
fn i32_div_signs(alt: bool) {
    let mut rng = 0x9e3779b97f4a7c15u64;
    let mut next = || { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; rng };
    for regime in ["positive", "signs", "wide"] {
        let data: Vec<(i32, i32)> = (0..1024).map(|_| {
            let a = if regime == "wide" { next() as i32 } else { (next() % 100_000_000) as i32 };
            let b = if regime == "wide" { next() as i32 } else { (next() % 65535 + 1) as i32 };
            if regime == "signs" { (a * if next() & 1 == 0 { -1 } else { 1 }, b * if next() & 1 == 0 { -1 } else { 1 }) } else { (a, b) }
        }).collect();
        // Compare the Option, not map_or(0), outside the timed loop. Zero divisors,
        // exact zero, ties, negative divisors and representable/overflow limits.
        for (a, b) in data.iter().copied().chain([(0,0),(0,-1),(1,32),(-1,32),(1,-32),(-1,-32),(i32::MIN,-1),(i32::MIN,10000),(i32::MAX,10000)]) {
            let reference = simple(a as i64 * 10000, b as i64).and_then(|q| i32::try_from(q).ok());
            let decimal = C::<i32,4>::from_mantissa(a).checked_div::<_,4,4>(C::from_mantissa(b)).map(|x|x.mantissa());
            assert_eq!(decimal, reference, "i32 div {a}/{b}");
        }
        let n = 15_000_000;
        let mut acc = 0u64;
        let start = Instant::now();
        for i in 0..n {
            let (a,b) = data[i & 1023];
            let q = if alt {
                simple(a as i64 * 10000, black_box(b) as i64).and_then(|q|i32::try_from(q).ok())
            } else {
                C::<i32,4>::from_mantissa(a).checked_div::<_,4,4>(black_box(C::from_mantissa(b))).map(|x|x.mantissa())
            };
            acc = acc.wrapping_add(q.unwrap_or(0) as u64);
        }
        black_box(acc);
        println!("{:<42} {:.2} ns/op", format!("i32 decimal div {regime}"), start.elapsed().as_nanos() as f64 / n as f64);
    }
}
