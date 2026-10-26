use div_pow10::bit128;

use crate::fpdec_inner::FpdecInner;
use crate::Rounding;

const SCALE_12: u128 = 10_u128.pow(12);

#[inline(always)]
fn checked_mul_u128(a: u128, b: u128) -> Option<u128> {
    // A fitting product must have at least one 64-bit operand.
    let (wide, word) = if let Ok(word) = u64::try_from(b) {
        (a, word)
    } else if let Ok(word) = u64::try_from(a) {
        (b, word)
    } else {
        return None;
    };
    let low = u128::from(wide as u64) * u128::from(word);
    // This sum is at most 2^128 - 2^64 - 1, so it cannot wrap.
    let high = (wide >> 64) * u128::from(word) + (low >> 64);
    let high = u64::try_from(high).ok()?;
    Some((u128::from(high) << 64) | u128::from(low as u64))
}

/// `n / 10^i` for `1 <= i <= 38`, for ANY `n: u128`.
///
/// `unchecked_div_single_r1b` is the faster form but requires `n <= 2^127`; the
/// reciprocal-path callers add a rounding `extra` of up to `10^i - 1` to a product only
/// known to be below `2^126`. For `i <= 37` that sum is below `2^126 + 10^37 < 2^127`
/// whatever the operands, so the fast form is always valid -- and testing `i` (a
/// compile-time constant for `ConstScaleFpdec`) costs nothing, where testing `n` itself
/// cost ~40% of an `i128` multiplication. Only `i = 38` can reach `2^127` (`10^38` is
/// about `2^126.2`), and only there is the value checked.
#[inline(always)]
fn div_pow10(n: u128, i: usize) -> u128 {
    // Keep this common divisor visible to LLVM in non-LTO consumers.
    if i == 12 {
        return n / SCALE_12;
    }
    // SAFETY: callers guarantee 1 <= i <= 38 (checked by `get_exp(i)`, and `i != 0`
    // by `calc_mul_div_exp`'s early return for `i == 0`).
    unsafe {
        if i < 38 || n <= 1_u128 << 127 {
            bit128::unchecked_div_single_r1b(n, i as u32)
        } else {
            bit128::unchecked_div_single(n, i as u32)
        }
    }
}

/// `a * b + extra` as the numerator for `div_pow10(_, i)`, or `None` when it may not fit `u128`
/// and the caller must use the double-word division instead.
///
/// `extra` is the rounding addend, below `10^i`.
#[inline(always)]
fn scaled_numerator(a: u128, b: u128, extra: u128, i: usize) -> Option<u128> {
    if (a | b) <= i64::MAX as u128 {
        // Both magnitudes fit 63 bits, so their product is below 2^126.
        Some(u128::from(a as u64) * u128::from(b as u64) + extra)
    } else if i == 12 {
        // Native division by 10^12 accepts any u128 numerator, so only overflow matters.
        checked_mul_u128(a, b).and_then(|product| product.checked_add(extra))
    } else if a.leading_zeros() + b.leading_zeros() >= 128 + 2 {
        // The product is below 2^126, so adding `extra < 10^38` cannot wrap. The sum may
        // exceed 2^127, which `div_pow10` handles.
        Some(a * b + extra)
    } else {
        None
    }
}

impl FpdecInner for i128 {
    const MAX: Self = Self::MAX;
    const MIN: Self = Self::MIN;
    const TEN: Self = 10;
    const HUNDRED: Self = 100;
    const MAX_POWERS: Self = 10_i128.pow(Self::DIGITS);
    const DIGITS: u32 = Self::MAX.ilog10();
    const NEG_MIN_STR: &'static str = "170141183460469231731687303715884105728";

    type Unsigned = u128;
    fn unsigned_abs(self) -> Self::Unsigned {
        self.unsigned_abs()
    }

    // Since we have our own calc_mul_div(), so we do not need these.
    type Wider = i128;
    fn as_wider(self) -> Self::Wider {
        unreachable!()
    }
    fn from_wider(_: Self::Wider) -> Option<Self> {
        unreachable!()
    }

    fn get_exp(i: usize) -> Option<Self> {
        const ALL_EXPS: [i128; 39] = [
            1,
            10_i128.pow(1),
            10_i128.pow(2),
            10_i128.pow(3),
            10_i128.pow(4),
            10_i128.pow(5),
            10_i128.pow(6),
            10_i128.pow(7),
            10_i128.pow(8),
            10_i128.pow(9),
            10_i128.pow(10),
            10_i128.pow(11),
            10_i128.pow(12),
            10_i128.pow(13),
            10_i128.pow(14),
            10_i128.pow(15),
            10_i128.pow(16),
            10_i128.pow(17),
            10_i128.pow(18),
            10_i128.pow(19),
            10_i128.pow(20),
            10_i128.pow(21),
            10_i128.pow(22),
            10_i128.pow(23),
            10_i128.pow(24),
            10_i128.pow(25),
            10_i128.pow(26),
            10_i128.pow(27),
            10_i128.pow(28),
            10_i128.pow(29),
            10_i128.pow(30),
            10_i128.pow(31),
            10_i128.pow(32),
            10_i128.pow(33),
            10_i128.pow(34),
            10_i128.pow(35),
            10_i128.pow(36),
            10_i128.pow(37),
            10_i128.pow(38),
        ];

        ALL_EXPS.get(i).copied()
    }

    #[inline(always)]
    fn calc_mul_div(self, b: Self, c: Self, rounding: Rounding) -> Option<Self> {
        let negative = (self ^ b ^ c) < 0;
        let rounding = match (negative, rounding) {
            (true, Rounding::Floor) => Rounding::Ceiling,
            (true, Rounding::Ceiling) => Rounding::TowardsZero,
            (_, rounding) => rounding,
        };
        let q = self
            .unsigned_abs()
            .calc_mul_div(b.unsigned_abs(), c.unsigned_abs(), rounding)?;
        if negative {
            if q > (1_u128 << 127) {
                return None;
            }
            Some((q as i128).wrapping_neg())
        } else {
            i128::try_from(q).ok()
        }
    }

    #[inline]
    fn calc_mul_div_exp(self, b: Self, i: usize, rounding: Rounding) -> Option<Self> {
        if i == 0 {
            return self.checked_mul(b);
        }
        let exp = Self::get_exp(i)?;

        let ua = self.unsigned_abs();
        let ub = b.unsigned_abs();

        let is_pos = self ^ b >= 0;

        #[rustfmt::skip]
        let extra = match rounding {
            Rounding::TowardsZero => 0,
            Rounding::AwayFromZero => exp - 1,
            Rounding::Round => exp / 2, // exp is even
            Rounding::Floor => if is_pos { 0 } else { exp - 1 },
            Rounding::Ceiling => if is_pos { exp - 1 } else { 0 },
        } as u128;

        let uq = if let Some(numerator) = scaled_numerator(ua, ub, extra, i) {
            div_pow10(numerator, i)
        } else {
            // use double-128-bit
            let (q, _) = bit128::mul_div(ua, ub, extra, i as u32)?;

            // check overflow
            if q > (i128::MAX as u128 + (!is_pos as u128)) {
                return None;
            }
            q
        };

        let iq = uq as i128;
        Some(if is_pos { iq } else { iq.wrapping_neg() })
    }

    #[inline(always)]
    fn rounding_div(self, b: Self, rounding: Rounding) -> Option<Self> {
        // try 64-bit first, which is much faster
        if let Ok(a64) = i64::try_from(self) {
            if let Ok(b64) = i64::try_from(b) {
                // may be overflow if: MIN / -1
                if let Some(q) = a64.rounding_div(b64, rounding) {
                    return Some(q as i128);
                }
            }
        }
        self.do_rounding_div(b, rounding)
    }
}

impl FpdecInner for u128 {
    const MAX: Self = Self::MAX;
    const MIN: Self = Self::MIN;
    const TEN: Self = 10;
    const HUNDRED: Self = 100;
    const MAX_POWERS: Self = 10_u128.pow(Self::DIGITS);
    const DIGITS: u32 = Self::MAX.ilog10();

    #[doc(hidden)]
    const NEG_MIN_STR: &'static str = "unreachable";

    type Unsigned = Self;
    fn unsigned_abs(self) -> Self::Unsigned {
        self
    }

    // Since we have our own calc_mul_div(), so we do not need these.
    type Wider = i128;
    fn as_wider(self) -> Self::Wider {
        unreachable!()
    }
    fn from_wider(_: Self::Wider) -> Option<Self> {
        unreachable!()
    }

    fn get_exp(i: usize) -> Option<Self> {
        const ALL_EXPS: [u128; 39] = [
            1,
            10_u128.pow(1),
            10_u128.pow(2),
            10_u128.pow(3),
            10_u128.pow(4),
            10_u128.pow(5),
            10_u128.pow(6),
            10_u128.pow(7),
            10_u128.pow(8),
            10_u128.pow(9),
            10_u128.pow(10),
            10_u128.pow(11),
            10_u128.pow(12),
            10_u128.pow(13),
            10_u128.pow(14),
            10_u128.pow(15),
            10_u128.pow(16),
            10_u128.pow(17),
            10_u128.pow(18),
            10_u128.pow(19),
            10_u128.pow(20),
            10_u128.pow(21),
            10_u128.pow(22),
            10_u128.pow(23),
            10_u128.pow(24),
            10_u128.pow(25),
            10_u128.pow(26),
            10_u128.pow(27),
            10_u128.pow(28),
            10_u128.pow(29),
            10_u128.pow(30),
            10_u128.pow(31),
            10_u128.pow(32),
            10_u128.pow(33),
            10_u128.pow(34),
            10_u128.pow(35),
            10_u128.pow(36),
            10_u128.pow(37),
            10_u128.pow(38),
        ];

        ALL_EXPS.get(i).copied()
    }

    #[inline(always)]
    fn calc_mul_div(self, b: Self, c: Self, rounding: Rounding) -> Option<Self> {
        // Expose the common 12dp factor and its exact product bound to LLVM.
        if b == SCALE_12 && self <= u128::MAX / SCALE_12 {
            return (self * SCALE_12).rounding_div(c, rounding);
        }
        // happy path, no overflow
        if let Some(r) = self.checked_mul(b) {
            return r.rounding_div(c, rounding);
        }

        calc_mul_div_wide(self, b, c, rounding)
    }

    #[inline]
    fn calc_mul_div_exp(self, b: Self, i: usize, rounding: Rounding) -> Option<Self> {
        if i == 0 {
            return self.checked_mul(b);
        }
        let exp = Self::get_exp(i)?;

        let extra = match rounding {
            Rounding::Floor | Rounding::TowardsZero => 0,
            Rounding::Ceiling | Rounding::AwayFromZero => exp - 1,
            Rounding::Round => exp / 2, // exp is even
        };

        if let Some(numerator) = scaled_numerator(self, b, extra, i) {
            Some(div_pow10(numerator, i))
        } else {
            bit128::mul_div(self, b, extra, i as u32).map(|(q, _r)| q)
        }
    }

    #[inline]
    fn rounding_div(self, b: Self, rounding: Rounding) -> Option<Self> {
        // try 64-bit first, which is much faster
        if let Ok(a64) = u64::try_from(self) {
            if let Ok(b64) = u64::try_from(b) {
                return a64.rounding_div(b64, rounding).map(|x| x as u128);
            }
        }
        self.do_rounding_div(b, rounding)
    }
}

// Keep the wide fallback out of consumers' common division loops.
#[inline(never)]
fn calc_mul_div_wide(a: u128, b: u128, c: u128, rounding: Rounding) -> Option<u128> {
    let (mhigh, mlow) = mul2(a, b);
    let (last_dividend, q) = if c < (1_u128 << 127) {
        reduce2(mhigh, mlow, c)?
    } else {
        reduce2_big(mhigh, mlow, c)?
    };
    let last_q = last_dividend.rounding_div(c, rounding)?;
    q.checked_add(last_q)
}

// calculate: a * b = (mhigh,mlow)
const fn mul2(a: u128, b: u128) -> (u128, u128) {
    let (ahigh, alow) = (a >> 64, a & u64::MAX as u128);
    let (bhigh, blow) = (b >> 64, b & u64::MAX as u128);

    let (mid, carry1) = (alow * bhigh).overflowing_add(ahigh * blow);
    let (mlow, carry2) = (alow * blow).overflowing_add(mid << 64);
    let mhigh = ahigh * bhigh + (mid >> 64) + ((carry1 as u128) << 64) + carry2 as u128;
    (mhigh, mlow)
}

// reduce: (mhigh, mlow) into (last_dividend, q) where last_dividend fits in 128-bit.
//
// calculate: (mhigh,mlow) / divisor = q .. last_dividend
// where last_dividend not {<divisor} while {fits in 128-bit}.
// So the caller should do the final division.
fn reduce2(mhigh: u128, mlow: u128, divisor: u128) -> Option<(u128, u128)> {
    // check overflow or c==0
    if mhigh >= divisor {
        return None;
    }

    // no need to reduce
    if mhigh == 0 {
        return Some((mlow, 0));
    }

    // consume @mhigh and reduce the dividend to @mlow
    let mut dividend = mhigh;
    let mut total_shft = 0;
    let mut q = 0;
    loop {
        let zeros = dividend.leading_zeros();
        debug_assert_ne!(zeros, 0); // because divisor < u128::MAX/2
        if zeros + total_shft >= 128 {
            break;
        }
        dividend = dividend << zeros | mlow << total_shft >> (128 - zeros);
        q = q << zeros | dividend / divisor;

        dividend %= divisor;
        total_shft += zeros;
    }

    q <<= 128 - total_shft;
    dividend = dividend << (128 - total_shft) | mlow << total_shft >> total_shft;

    Some((dividend, q))
}

// Works for divisor >= u128::MAX/2
// We can not use division in loop, but use substraction.
#[cold]
fn reduce2_big(mhigh: u128, mlow: u128, divisor: u128) -> Option<(u128, u128)> {
    // check overflow or c==0
    if mhigh >= divisor {
        return None;
    }

    // no need to reduce
    if mhigh == 0 {
        return Some((mlow, 0));
    }

    // consume @mhigh and reduce the dividend to @mlow
    let mut dividend = mhigh;
    let mut total_shft = 0;
    let mut q: u128 = 0;
    loop {
        let mut zeros = dividend.leading_zeros();
        if zeros + total_shft >= 128 {
            break;
        }

        // here 'zeros' may be 0
        if zeros != 0 {
            dividend = dividend << zeros | mlow << total_shft >> (128 - zeros);
        }

        // shift 1 more bit to make sure zeros>0, if need
        if dividend < divisor {
            dividend = dividend << 1 | mlow << (total_shft + zeros) >> 127;
            zeros += 1;
        }

        // 'zeros' may be 128 because of the 1 more bit above,
        // this may happen only at the first loop where 'q'=0.
        q = q.unbounded_shl(zeros) + 1;

        // using 'wrapping_sub' because of the 1 more bit above.
        dividend = dividend.wrapping_sub(divisor);

        total_shft += zeros;
    }

    if total_shft < 128 {
        q <<= 128 - total_shft;
        dividend = dividend << (128 - total_shft) | mlow << total_shft >> total_shft;
    }

    Some((dividend, q))
}

#[cfg(test)]
mod tests {
    #[test]
    fn checked_word_product_matches_integer_overflow_contract() {
        let values = [
            0,
            1,
            2,
            3,
            (1_u128 << 32) - 1,
            1 << 32,
            (1 << 63) - 1,
            1 << 63,
            u64::MAX as u128 - 1,
            u64::MAX as u128,
            u64::MAX as u128 + 1,
            u64::MAX as u128 + 2,
            u64::MAX as u128 + 3,
            (1 << 127) - 1,
            1 << 127,
            u128::MAX,
        ];
        for a in values {
            for b in values {
                assert_eq!(super::checked_mul_u128(a, b), a.checked_mul(b), "{a} * {b}");
            }
        }
        for b in [2, 3, 5, 1 << 32, (1 << 63) + 1, u64::MAX as u128] {
            let limit = u128::MAX / b;
            for a in [limit - 1, limit, limit + 1] {
                assert_eq!(super::checked_mul_u128(a, b), a.checked_mul(b), "{a} * {b}");
                assert_eq!(super::checked_mul_u128(b, a), b.checked_mul(a), "{b} * {a}");
            }
        }
    }
}
