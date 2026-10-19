use div_pow10::bit64;

use crate::fpdec_inner::FpdecInner;
use crate::Rounding;

macro_rules! common_consts {
    ($wider_typ:ty) => {
        const MAX: Self = Self::MAX;
        const MIN: Self = Self::MIN;
        const TEN: Self = 10;
        const HUNDRED: Self = 100;
        const MAX_POWERS: Self = Self::TEN.pow(Self::DIGITS);
        const DIGITS: u32 = Self::MAX.ilog10();

        type Wider = $wider_typ;
        fn as_wider(self) -> Self::Wider {
            self as $wider_typ
        }
        fn from_wider(w: Self::Wider) -> Option<Self> {
            Self::try_from(w).ok()
        }
    };
}

macro_rules! signed_consts {
    ($wider_typ:ty, $uns_typ:ty, $neg_min_str:expr) => {
        common_consts!($wider_typ);

        const NEG_MIN_STR: &'static str = $neg_min_str;

        type Unsigned = $uns_typ;
        fn unsigned_abs(self) -> Self::Unsigned {
            self.unsigned_abs()
        }
    };
}

macro_rules! unsigned_consts {
    ($wider_typ:ty) => {
        common_consts!($wider_typ);

        #[doc(hidden)]
        const NEG_MIN_STR: &'static str = "unreachable";

        type Unsigned = Self;
        fn unsigned_abs(self) -> Self::Unsigned {
            self
        }
    };
}

impl FpdecInner for i8 {
    signed_consts!(i16, u8, "128");

    fn get_exp(i: usize) -> Option<Self> {
        const ALL_EXPS: [i8; 3] = [1, 10_i8.pow(1), 10_i8.pow(2)];

        ALL_EXPS.get(i).copied()
    }
}

impl FpdecInner for i16 {
    signed_consts!(i32, u16, "32768");

    fn get_exp(i: usize) -> Option<Self> {
        const ALL_EXPS: [i16; 5] = [
            1,
            10_i16.pow(1),
            10_i16.pow(2),
            10_i16.pow(3),
            10_i16.pow(4),
        ];

        ALL_EXPS.get(i).copied()
    }
}

impl FpdecInner for i32 {
    signed_consts!(i64, u32, "2147483648");

    fn get_exp(i: usize) -> Option<Self> {
        const ALL_EXPS: [i32; 10] = [
            1,
            10_i32.pow(1),
            10_i32.pow(2),
            10_i32.pow(3),
            10_i32.pow(4),
            10_i32.pow(5),
            10_i32.pow(6),
            10_i32.pow(7),
            10_i32.pow(8),
            10_i32.pow(9),
        ];

        ALL_EXPS.get(i).copied()
    }

    // The product fits i64. Add the rounding extra to its magnitude before
    // dividing: this avoids a signed quotient/remainder pair on the hot path.
    #[inline]
    fn calc_mul_div_exp(self, b: Self, i: usize, rounding: Rounding) -> Option<Self> {
        let exp = Self::get_exp(i)? as u64;
        let p = self as i64 * b as i64;
        let negative = p < 0;
        let extra = match rounding {
            Rounding::TowardsZero => 0,
            Rounding::AwayFromZero => exp - 1,
            Rounding::Floor => if negative { exp - 1 } else { 0 },
            Rounding::Ceiling => if negative { 0 } else { exp - 1 },
            Rounding::Round => exp / 2,
        };
        // |p| <= 2^62 and extra < 10^9, so the addition cannot overflow u64.
        let q = (p.unsigned_abs() + extra) / exp;
        if negative {
            (q <= 1 << 31).then(|| (q as u32).wrapping_neg() as i32)
        } else {
            i32::try_from(q).ok()
        }
    }
}

impl FpdecInner for i64 {
    signed_consts!(i128, u64, "9223372036854775808");

    fn get_exp(i: usize) -> Option<Self> {
        const ALL_EXPS: [i64; 19] = [
            1,
            10_i64.pow(1),
            10_i64.pow(2),
            10_i64.pow(3),
            10_i64.pow(4),
            10_i64.pow(5),
            10_i64.pow(6),
            10_i64.pow(7),
            10_i64.pow(8),
            10_i64.pow(9),
            10_i64.pow(10),
            10_i64.pow(11),
            10_i64.pow(12),
            10_i64.pow(13),
            10_i64.pow(14),
            10_i64.pow(15),
            10_i64.pow(16),
            10_i64.pow(17),
            10_i64.pow(18),
        ];

        ALL_EXPS.get(i).copied()
    }

    // The default goes through a signed `i128` division, which is two library calls (quotient
    // and remainder) plus sign fix-ups. On the magnitudes it is one `u128 / u64`.
    #[inline]
    fn calc_mul_div(self, b: Self, c: Self, rounding: Rounding) -> Option<Self> {
        let uc = c.unsigned_abs();
        if uc == 0 {
            return None;
        }
        let is_neg = (self ^ b ^ c) < 0;

        let p = self.unsigned_abs() as u128 * b.unsigned_abs() as u128;
        let (q, r) = match u64::try_from(p) {
            Ok(p) => ((p / uc) as u128, p % uc),
            Err(_) => (p / uc as u128, (p % uc as u128) as u64),
        };

        // `r < uc`, so neither `uc - r` nor `q + 1` can overflow.
        let up = r != 0
            && match rounding {
                Rounding::TowardsZero => false,
                Rounding::AwayFromZero => true,
                Rounding::Floor => is_neg,
                Rounding::Ceiling => !is_neg,
                Rounding::Round => r >= uc - r,
            };
        let q = q + up as u128;

        if is_neg {
            (q <= 1 << 63).then(|| (q as u64).wrapping_neg() as i64)
        } else {
            i64::try_from(q).ok()
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
        };

        let uq = if ua.leading_zeros() + ub.leading_zeros() >= 64 + 2 {
            // happy path, (self * b + extra) is not overflow 64-bit
            let divident = ua * ub + extra as u64;

            // SAFETY:
            //   i != 0, checked in checked_mul_ext() outside
            //   i <= 19, checked by get_exp() before
            unsafe { bit64::unchecked_div_single(divident, i as u32) }
        } else {
            // use 128-bit
            let divident = ua as u128 * ub as u128 + extra as u128;
            let (q, _) = bit64::div_double(divident, i as u32)?;

            // check overflow
            if q > (i64::MAX as u64 + (!is_pos as u64)) {
                return None;
            }
            q
        };

        let iq = uq as i64;
        Some(if is_pos { iq } else { iq.wrapping_neg() })
    }
}

impl FpdecInner for u8 {
    unsigned_consts!(u16);

    fn get_exp(i: usize) -> Option<Self> {
        const ALL_EXPS: [u8; 3] = [1, 10_u8.pow(1), 10_u8.pow(2)];

        ALL_EXPS.get(i).copied()
    }
}

impl FpdecInner for u16 {
    unsigned_consts!(u32);

    fn get_exp(i: usize) -> Option<Self> {
        const ALL_EXPS: [u16; 5] = [
            1,
            10_u16.pow(1),
            10_u16.pow(2),
            10_u16.pow(3),
            10_u16.pow(4),
        ];

        ALL_EXPS.get(i).copied()
    }
}

impl FpdecInner for u32 {
    unsigned_consts!(u64);

    fn get_exp(i: usize) -> Option<Self> {
        const ALL_EXPS: [u32; 10] = [
            1,
            10_u32.pow(1),
            10_u32.pow(2),
            10_u32.pow(3),
            10_u32.pow(4),
            10_u32.pow(5),
            10_u32.pow(6),
            10_u32.pow(7),
            10_u32.pow(8),
            10_u32.pow(9),
        ];

        ALL_EXPS.get(i).copied()
    }
}

impl FpdecInner for u64 {
    unsigned_consts!(u128);

    fn get_exp(i: usize) -> Option<Self> {
        const ALL_EXPS: [u64; 20] = [
            1,
            10_u64.pow(1),
            10_u64.pow(2),
            10_u64.pow(3),
            10_u64.pow(4),
            10_u64.pow(5),
            10_u64.pow(6),
            10_u64.pow(7),
            10_u64.pow(8),
            10_u64.pow(9),
            10_u64.pow(10),
            10_u64.pow(11),
            10_u64.pow(12),
            10_u64.pow(13),
            10_u64.pow(14),
            10_u64.pow(15),
            10_u64.pow(16),
            10_u64.pow(17),
            10_u64.pow(18),
            10_u64.pow(19),
        ];

        ALL_EXPS.get(i).copied()
    }

    // See the `i64` one.
    #[inline]
    fn calc_mul_div(self, b: Self, c: Self, rounding: Rounding) -> Option<Self> {
        if c == 0 {
            return None;
        }
        let p = self as u128 * b as u128;
        let (q, r) = match u64::try_from(p) {
            Ok(p) => ((p / c) as u128, p % c),
            Err(_) => (p / c as u128, (p % c as u128) as u64),
        };

        // `r < c`, so neither `c - r` nor `q + 1` can overflow.
        let up = r != 0
            && match rounding {
                Rounding::Floor | Rounding::TowardsZero => false,
                Rounding::Ceiling | Rounding::AwayFromZero => true,
                Rounding::Round => r >= c - r,
            };
        u64::try_from(q + up as u128).ok()
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

        if self.leading_zeros() + b.leading_zeros() >= 64 + 1 {
            // happy path: `self * b < 2^63`. `extra` is at most `10^i - 1`, so for
            // `i <= 18` the sum stays below `2^63 + 10^18 < 2^64`. Only `i = 19`
            // (`Ceiling` / `AwayFromZero`, `extra` up to `10^19 - 1`) can pass `u64::MAX`:
            // there the add is checked, where `+` would panic in debug and wrap to a
            // wrong quotient in release. Testing `i` rather than the sum is free for
            // `ConstScaleFpdec`, whose `i` is a constant.
            let prod = self * b;
            let sum = if i < 19 { Some(prod + extra) } else { prod.checked_add(extra) };
            if let Some(divident) = sum {
                // SAFETY:
                //   i != 0, checked in checked_mul_ext() outside
                //   i <= 19, checked by get_exp() before
                return unsafe { Some(bit64::unchecked_div_single(divident, i as u32)) };
            }
        }

        let divident = self as u128 * b as u128 + extra as u128;
        bit64::div_double(divident, i as u32).map(|(q, _r)| q)
    }
}
