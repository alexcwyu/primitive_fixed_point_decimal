use crate::{ParseError, Rounding};

use core::{
    fmt,
    mem::MaybeUninit,
    num::{IntErrorKind, ParseIntError},
    ops::{AddAssign, SubAssign},
};

use num_traits::{
    identities::{ConstOne, ConstZero, Zero},
    int::PrimInt,
    ops::wrapping::WrappingAdd,
    AsPrimitive, Num,
};

/// The trait for underlying representation.
///
/// Normal users don't need to use this trait.
pub trait FpdecInner:
    PrimInt + ConstOne + ConstZero + AddAssign + SubAssign + WrappingAdd + Zero
{
    const MAX: Self;
    const MIN: Self;
    const TEN: Self;
    const HUNDRED: Self;
    const MAX_POWERS: Self;
    const DIGITS: u32;
    const NEG_MIN_STR: &'static str;

    /// Used by unsigned_abs() method.
    type Unsigned: FpdecInner + AsPrimitive<u8> + AsPrimitive<usize>;

    /// For signed types, this should call their unsigned_abs();
    /// for unsigned types, this should return self directly.
    fn unsigned_abs(self) -> Self::Unsigned;

    /// Return 10 to the power of `i`.
    fn get_exp(i: usize) -> Option<Self>;

    /// Used by as_wider() and from_wider(), which are invoked by calc_mul_div().
    ///
    /// `i128` and `u128` have no wider type: they implement `calc_mul_div()`
    /// themselves and `as_wider()` / `from_wider()` panic for them.
    type Wider: FpdecInner;

    /// Convert to [`Self::Wider`]. Panics for `i128` and `u128`.
    ///
    /// ```
    /// use primitive_fixed_point_decimal::FpdecInner;
    /// assert_eq!((-3_i32).as_wider(), -3_i64);
    /// ```
    ///
    /// ```should_panic
    /// use primitive_fixed_point_decimal::FpdecInner;
    /// let _ = 3_i128.as_wider(); // no wider type
    /// ```
    fn as_wider(self) -> Self::Wider;

    /// Convert from [`Self::Wider`], `None` if it does not fit. Panics for
    /// `i128` and `u128`.
    fn from_wider(w: Self::Wider) -> Option<Self>;

    /// Calculate `self * b / c`.
    /// Since i128 and u128 do not have wider types, then have
    /// their own implementations.
    fn calc_mul_div(self, b: Self, c: Self, rounding: Rounding) -> Option<Self> {
        let p = self.as_wider() * b.as_wider();
        p.rounding_div(c.as_wider(), rounding)
            .and_then(Self::from_wider)
    }

    /// Calculate `self * b / EXP[i]`.
    /// i32, i64, u64, i128, u128 have their own implemetations to avoid the
    /// expensive 128-bit division.
    fn calc_mul_div_exp(self, b: Self, i: usize, rounding: Rounding) -> Option<Self> {
        self.calc_mul_div(b, Self::get_exp(i)?, rounding)
    }

    /// Calculate `self / b / EXP[i]`.
    fn calc_div_div_exp(self, b: Self, i: usize, rounding: Rounding) -> Option<Self> {
        let exp = Self::get_exp(i)?;
        match b.checked_mul(&exp) {
            Some(p) => self.rounding_div(p, rounding),
            None => {
                let is_pos = self ^ b >= Self::ZERO;
                let c = match rounding {
                    Rounding::TowardsZero => false,
                    Rounding::AwayFromZero => self != Self::ZERO,
                    Rounding::Floor => {
                        if is_pos {
                            false
                        } else {
                            self != Self::ZERO
                        }
                    }
                    Rounding::Ceiling => {
                        if !is_pos {
                            false
                        } else {
                            self != Self::ZERO
                        }
                    }
                    Rounding::Round => match b.checked_mul(&(exp >> 1)) {
                        Some(p) => self.unsigned_abs() >= p.unsigned_abs(),
                        None => false,
                    },
                };
                if !c {
                    Self::ZERO
                } else if is_pos {
                    Self::ONE
                } else {
                    Self::ZERO - Self::ONE
                }
            }
            .into(), // Some()
        }
    }

    // works only when: diff_scale in range [-Self::DIGITS, Self::DIGITS]
    // diff_scale = scale (self + rhs - result)
    #[inline(always)]
    fn checked_mul_ext(self, rhs: Self, diff_scale: i32, rounding: Rounding) -> Option<Self> {
        // Keep the common 128-bit 12dp exponent constant for runtime OOB scales.
        if Self::DIGITS == 38 && diff_scale == 12 {
            return self.calc_mul_div_exp(rhs, 12, rounding);
        }
        if diff_scale > 0 {
            // self * rhs / diff_exp

            // If diff_scale is in range [Self::DIGITS+1, Self::DIGITS*2], we
            // could do division twice (with exp[DIGITS] and exp[diff_scale-DIGITS])
            // to avoid returning `None` directly, but that's not enough.
            // Because `MAX * MAX / exp[DIGITS]` still overflows. For
            // simplicity's sake, we do not handle this case which is rare.
            self.calc_mul_div_exp(rhs, diff_scale as usize, rounding)
        } else if diff_scale < 0 {
            // self * rhs * diff_exp
            let exp = Self::get_exp(diff_scale.unsigned_abs() as usize)?;
            self.checked_mul(&rhs)?.checked_mul(&exp)
        } else {
            self.checked_mul(&rhs)
        }
    }

    // works only when: diff_scale in range [-Self::DIGITS, Self::DIGITS]
    // diff_scale = scale (self - rhs - result)
    // Keep constant scales/rounding visible through the mixed-type dispatcher.
    #[inline(always)]
    fn checked_div_ext(self, rhs: Self, diff_scale: i32, rounding: Rounding) -> Option<Self> {
        // Keep the common 128-bit 12dp factor constant for runtime OOB scales.
        if Self::DIGITS == 38 && diff_scale == -12 {
            return self.calc_mul_div(Self::get_exp(12)?, rhs, rounding);
        }
        if diff_scale > 0 {
            // self / rhs / diff_exp
            self.calc_div_div_exp(rhs, diff_scale as usize, rounding)
        } else if diff_scale < 0 {
            // self * diff_exp / rhs

            // If diff_scale is in range [-Self::DIGITS*2, -Self::DIGITS-1], we
            // could do multiplication twice (with exp[DIGITS] and exp[-diff_scale-DIGITS])
            // to avoid returning `None` directly. But keep same with
            // `checked_mul()`, we do not handle this case which is rare.
            let exp = Self::get_exp(diff_scale.unsigned_abs() as usize)?;
            self.calc_mul_div(exp, rhs, rounding)
        } else {
            self.rounding_div(rhs, rounding)
        }
    }

    // diff_scale = scale (src - dst)
    //
    // Returns `None` if the rounded value is not representable: `i8` 127 rounded
    // UP to hundreds is 200, and rounding any non-zero value away from zero at a
    // scale beyond `Self::DIGITS` gives `+-10^diff_scale`.
    fn checked_round_diff_with_rounding(self, diff_scale: i32, rounding: Rounding) -> Option<Self> {
        if diff_scale <= 0 {
            return Some(self);
        }

        match Self::get_exp(diff_scale as usize) {
            // `10^diff_scale` is bigger than every value of the type, so the
            // quotient is 0 or +-1. A quotient of +-1 would be `+-10^diff_scale`
            // itself, which is not representable.
            None => {
                if self.rounds_to_unit_beyond_range(diff_scale as usize, rounding) {
                    None
                } else {
                    Some(Self::ZERO)
                }
            }
            // self / exp * exp
            Some(exp) => self.rounding_div(exp, rounding)?.checked_mul(&exp),
        }
    }

    // diff_scale = scale (src - dst): the same number at another scale. Rounds the digits
    // dropped when `diff_scale > 0`; `None` if it does not fit when `diff_scale < 0`.
    // Inline through the wrappers before optimising division: a constant scale
    // must reach the i128 rounding_div fast path as a constant divisor.
    #[doc(hidden)]
    #[inline(always)]
    fn checked_rescale_diff(self, diff_scale: i64, rounding: Rounding) -> Option<Self> {
        if diff_scale == 0 {
            Some(self)
        } else if diff_scale > 0 {
            let i = usize::try_from(diff_scale).unwrap_or(usize::MAX);
            match Self::get_exp(i) {
                Some(exp) => self.rounding_div(exp, rounding),
                // `|self| < 10^i`: the quotient is 0 or +-1
                None => Some(if !self.rounds_to_unit_beyond_range(i, rounding) {
                    Self::ZERO
                } else if self < Self::ZERO {
                    Self::ZERO - Self::ONE
                } else {
                    Self::ONE
                }),
            }
        } else if diff_scale.unsigned_abs() > Self::DIGITS as u64 && self.is_zero() {
            Some(self) // zero is zero at every scale, `10^-diff_scale` need not fit
        } else {
            let i = usize::try_from(diff_scale.unsigned_abs()).ok()?;
            self.checked_mul(&Self::get_exp(i)?)
        }
    }

    /// For `|self| < 10^i` where `10^i` does NOT fit in `Self` (`i > DIGITS`):
    /// does `self / 10^i` round to a non-zero value (`+-1`)?
    fn rounds_to_unit_beyond_range(self, i: usize, rounding: Rounding) -> bool {
        if self.is_zero() {
            return false;
        }
        match rounding {
            Rounding::TowardsZero => false,
            Rounding::AwayFromZero => true,
            Rounding::Floor => self < Self::ZERO,
            Rounding::Ceiling => self > Self::ZERO,
            Rounding::Round => {
                // `2 * |self| >= 10^i`. Since `Self::MAX < 10^(DIGITS+1)`, that can
                // hold only for `i == DIGITS + 1`, where it reads
                // `|self| >= 5 * 10^DIGITS` -- reachable for `u16` (65535 >= 50000)
                // and `i64` (MAX >= 5e18), and impossible for the others. Divide instead of multiplying so
                // nothing can overflow.
                let five = <Self::Unsigned as num_traits::NumCast>::from(5u8).unwrap();
                i == Self::DIGITS as usize + 1
                    && self.unsigned_abs() / Self::MAX_POWERS.unsigned_abs() >= five
            }
        }
    }

    /// Calculate rounding division.
    ///
    /// Examples:
    ///
    /// ```
    /// use primitive_fixed_point_decimal::{FpdecInner, Rounding};
    /// assert_eq!(8.rounding_div(3, Rounding::Floor), Some(2));
    /// assert_eq!(8.rounding_div(3, Rounding::Round), Some(3));
    /// assert_eq!(8.rounding_div(3, Rounding::Ceiling), Some(3));
    /// assert_eq!(8.rounding_div(-3, Rounding::Floor), Some(-3));
    /// assert_eq!(8.rounding_div(-3, Rounding::Round), Some(-3));
    /// assert_eq!(8.rounding_div(-3, Rounding::Ceiling), Some(-2));
    /// assert_eq!((-23).rounding_div(-5, Rounding::Round), Some(5));
    /// assert_eq!((-21).rounding_div(-5, Rounding::Round), Some(4));
    /// assert_eq!(120_i8.rounding_div(121_i8, Rounding::Round), Some(1));
    /// assert_eq!(120_i8.rounding_div(-121_i8, Rounding::Round), Some(-1));
    /// ```
    fn rounding_div(self, b: Self, rounding: Rounding) -> Option<Self> {
        self.do_rounding_div(b, rounding)
    }

    fn do_rounding_div(self, b: Self, rounding: Rounding) -> Option<Self> {
        let q = self.checked_div(&b)?;
        if rounding == Rounding::TowardsZero {
            return Some(q);
        }
        let remain = self % b;
        if remain == Self::ZERO {
            return Some(q);
        }

        if (self ^ b) >= Self::ZERO {
            // unsigned types, or signed types and self and b have same sign
            match rounding {
                Rounding::Floor | Rounding::TowardsZero => q,
                Rounding::Ceiling | Rounding::AwayFromZero => q + Self::ONE,
                Rounding::Round => {
                    if b >= Self::ZERO {
                        if remain >= b - remain {
                            q + Self::ONE
                        } else {
                            q
                        }
                    } else {
                        if remain <= b - remain {
                            q + Self::ONE
                        } else {
                            q
                        }
                    }
                }
            }
        } else {
            // signed types and self and b have different sign.
            match rounding {
                Rounding::Floor | Rounding::AwayFromZero => q - Self::ONE,
                Rounding::Ceiling | Rounding::TowardsZero => q,
                Rounding::Round => {
                    let r = remain.unsigned_abs();
                    if r >= b.unsigned_abs() - r {
                        q - Self::ONE
                    } else {
                        q
                    }
                }
            }
        }
        .into() // Some()
    }

    // INTERNAL
    // Parse an string as negative.
    // We try to parse it as positive first. If fail for overflow,
    // then it maybe the MIN value.
    fn parse_int_as_negative(s: &str) -> Result<Self, ParseIntError>
    where
        Self: Num<FromStrRadixErr = ParseIntError>,
    {
        match Self::from_str_radix(s, 10) {
            Ok(num) => {
                // Return -num.
                // The FpdecInner works for both signed and unsigned types.
                // But unsigned type does not support negative operation.
                // So we use Two's Complement to calculate negative.
                Ok((!num).wrapping_add(&Self::ONE))
            }
            Err(err) => {
                if err.kind() == &IntErrorKind::PosOverflow
                    && s.trim_start_matches('0') == Self::NEG_MIN_STR
                {
                    Ok(Self::MIN)
                } else {
                    Err(err)
                }
            }
        }
    }

    fn try_from_str(s: &str, scale: i32) -> Result<Self, ParseError>
    where
        Self: Num<FromStrRadixErr = ParseIntError>,
    {
        match Self::try_from_ascii_fast(s.as_bytes(), scale) {
            Some(v) => Ok(v),
            None => Self::try_from_str_exact(s, scale),
        }
    }

    // Same as `try_from_str()` for bytes: market data arrives as `&[u8]`, and validating UTF-8
    // first just to take the string back apart costs more than the parse. Bytes that are not
    // UTF-8 are `Invalid`, as any non-digit is.
    #[doc(hidden)]
    fn try_from_ascii(b: &[u8], scale: i32) -> Result<Self, ParseError>
    where
        Self: Num<FromStrRadixErr = ParseIntError>,
    {
        match Self::try_from_ascii_fast(b, scale) {
            Some(v) => Ok(v),
            None => match core::str::from_utf8(b) {
                Ok(s) => Self::try_from_str_exact(s, scale),
                Err(_) => Err(ParseError::Invalid),
            },
        }
    }

    // INTERNAL
    // The common case of `try_from_str()` in one pass over the bytes: `[+-]?digits('.'digits)?`
    // with at most `Self::DIGITS` digits in all (so the accumulator cannot overflow) and a
    // fraction no longer than `scale`. Everything else -- other spellings, long numbers, every
    // error -- returns `None`, and `try_from_str_exact()` decides it. So the two can never
    // disagree about an error, and the tests check that they agree on every value.
    #[inline]
    #[doc(hidden)]
    fn try_from_ascii_fast(b: &[u8], scale: i32) -> Option<Self> {
        let (neg, mut i) = match *b.first()? {
            b'-' => (true, 1),
            b'+' => (false, 1),
            _ => (false, 0),
        };
        if neg && Self::MIN == Self::ZERO {
            return None; // unsigned: "-0" is zero but "-1" is an error; let the exact path say
        }

        let max_digits = Self::DIGITS as usize;
        let mut acc = Self::ZERO;
        let mut n_digits = 0;

        while let Some(&c) = b.get(i) {
            let d = c.wrapping_sub(b'0');
            if d > 9 {
                break;
            }
            if n_digits == max_digits {
                return None;
            }
            acc = acc * Self::TEN + <Self as num_traits::NumCast>::from(d)?;
            n_digits += 1;
            i += 1;
        }
        if n_digits == 0 {
            return None;
        }

        let mut n_frac = 0;
        if i < b.len() {
            if b[i] != b'.' {
                return None;
            }
            i += 1;
            while let Some(&c) = b.get(i) {
                let d = c.wrapping_sub(b'0');
                if d > 9 {
                    return None;
                }
                if n_digits == max_digits {
                    return None;
                }
                acc = acc * Self::TEN + <Self as num_traits::NumCast>::from(d)?;
                n_digits += 1;
                n_frac += 1;
                i += 1;
            }
            if n_frac == 0 {
                return None; // "1."
            }
        }

        // `10^shift` for the missing fraction digits. A fraction longer than `scale` is a
        // precision error, and an integer string at a negative scale needs its trailing
        // zeros counted; both are left to the exact path.
        let shift = scale as i64 - n_frac as i64;
        if shift < 0 {
            return None;
        }

        // Signed first, then scaled: `-MIN` is not representable but `MIN * 10^shift` is
        // checked like any other product.
        let v = if neg { Self::ZERO - acc } else { acc };
        if shift == 0 {
            Some(v)
        } else {
            v.checked_mul(&Self::get_exp(shift as usize)?)
        }
    }

    // INTERNAL
    // The complete implementation of `try_from_str()`, with its error kinds.
    #[doc(hidden)]
    fn try_from_str_exact(s: &str, scale: i32) -> Result<Self, ParseError>
    where
        Self: Num<FromStrRadixErr = ParseIntError>,
    {
        let (num, raw_scale) = match Self::try_from_str_only(s) {
            Ok(parsed) => parsed,
            // The digits alone do not fit in `Self`, and `try_from_str_only` cannot
            // say whether that is a precision or an overflow problem because it
            // does not know `scale`. More fraction digits than `scale` is
            // precision; otherwise the number is simply too big.
            Err(ParseError::Overflow | ParseError::Precision) => {
                return match s.split_once('.') {
                    Some((_, frac)) if frac.len() as i64 > scale as i64 => Err(ParseError::Precision),
                    _ => Err(ParseError::Overflow),
                };
            }
            Err(e) => return Err(e),
        };
        if num.is_zero() {
            Ok(num)
        } else if raw_scale == scale {
            Ok(num)
        } else if raw_scale > scale {
            Err(ParseError::Precision)
        } else {
            // `num` is non-zero, so if `10^(scale - raw_scale)` itself does not fit
            // then neither does the product.
            // (i64: `scale - raw_scale` overflows i32 for scales near the i32 limits)
            Self::get_exp((scale as i64 - raw_scale as i64) as usize)
                .ok_or(ParseError::Overflow)?
                .checked_mul(&num)
                .ok_or(ParseError::Overflow)
        }
    }

    // Guess and return the scale by the input string.
    fn try_from_str_only(s: &str) -> Result<(Self, i32), ParseError>
    where
        Self: Num<FromStrRadixErr = ParseIntError>,
    {
        if s.is_empty() {
            return Err(ParseError::Empty);
        }

        if let Some((int_str, frac_str)) = s.split_once('.') {
            // The fraction is plain digits. `from_str_radix` would also take a sign
            // there, so "1.+5" parsed as 1.05, "1.-5" as 0.95 and "0.-5" as -0.05.
            if frac_str.is_empty() {
                return Err(ParseError::Empty);
            }
            if !frac_str.bytes().all(|b| b.is_ascii_digit()) {
                return Err(ParseError::Invalid);
            }

            // Any spelling of zero ("0.0", "-0.00", "+00.0"), for every inner type.
            // `from_str_radix("-0")` is an error for unsigned types.
            let int_digits = int_str.strip_prefix(|c| c == '+' || c == '-').unwrap_or(int_str);
            if !int_digits.is_empty()
                && int_digits.bytes().all(|b| b == b'0')
                && frac_str.bytes().all(|b| b == b'0')
            {
                return Ok((Self::ZERO, frac_str.len() as i32));
            }

            let int_num = Self::from_str_radix(int_str, 10)?;

            let frac_num = if s.as_bytes()[0] == b'-' {
                Self::parse_int_as_negative(frac_str)?
            } else {
                Self::from_str_radix(frac_str, 10)?
            };

            let inner = if int_num.is_zero() {
                // only fraction part
                frac_num
            } else {
                // exp * integer + fraction
                Self::get_exp(frac_str.len())
                    .ok_or(ParseError::Precision)?
                    .checked_mul(&int_num)
                    .ok_or(ParseError::Overflow)?
                    .checked_add(&frac_num)
                    .ok_or(ParseError::Overflow)?
            };
            Ok((inner, frac_str.len() as i32))
        } else {
            // only integer part. Any spelling of zero: "0", "-0", "+0", "000", "-00".
            // (Without this, "00" was `Empty`, because trimming its trailing zeros
            // below leaves nothing to parse.)
            let digits = s.strip_prefix(|c| c == '+' || c == '-').unwrap_or(s);
            if !digits.is_empty() && digits.bytes().all(|b| b == b'0') {
                return Ok((Self::ZERO, 0));
            }
            let new_int_str = s.trim_end_matches('0');
            let diff = s.len() - new_int_str.len();
            Ok((Self::from_str_radix(new_int_str, 10)?, -(diff as i32)))
        }
    }

    fn display_fmt(self, scale: i32, f: &mut fmt::Formatter) -> fmt::Result {
        // The buffer is 250 long. 50 is for the number, and 200 is for
        // the padding zeros for specified precision and big scales.
        // We panic if the string is too long.
        let mut buf: [MaybeUninit<u8>; 250] = [MaybeUninit::uninit(); 250];
        // `unsigned_abs`: `i32::MIN.abs()` overflows (and wraps negative in release builds,
        // which would sail past this check).
        assert!(scale.unsigned_abs() <= 200);

        let (start, end) = display_num(self.unsigned_abs(), scale, f.precision(), &mut buf);

        // SAFETY: `display_num` initialized every byte of `buf[start..end]`.
        let buf = unsafe {
            core::slice::from_raw_parts(buf[start..end].as_ptr().cast(), end - start)
        };

        // SAFETY: all data is valid charactor
        let s = unsafe { str::from_utf8_unchecked(buf) };

        f.pad_integral(self >= Self::ZERO, "", s)
    }

    // Format like `display_fmt()` (the sign included, no width or fill) into `out`, without
    // going through `core::fmt`, whose machinery is most of the time of formatting a number.
    // `None` if the text does not fit `out`, or the scale or precision is beyond what the
    // formatting supports (the limits that make `display_fmt()` panic).
    #[doc(hidden)]
    fn format_into(self, scale: i32, precision: Option<usize>, out: &mut [u8]) -> Option<&str> {
        let too_long = match precision {
            None => false,
            Some(p) if scale > 0 => p > 200,
            Some(p) => p != 0 && p > 200_usize.saturating_sub(scale.unsigned_abs() as usize),
        };
        if scale.unsigned_abs() > 200 || too_long {
            return None;
        }

        let mut buf: [MaybeUninit<u8>; 250] = [MaybeUninit::uninit(); 250];
        let (start, end) = display_num(self.unsigned_abs(), scale, precision, &mut buf);

        let is_neg = self < Self::ZERO;
        let len = end - start + is_neg as usize;
        let out = out.get_mut(..len)?;
        if is_neg {
            out[0] = b'-';
        }
        // SAFETY: `display_num` initialized every byte of `buf[start..end]`, and the
        // destination `out[is_neg..]` is exactly `end - start` bytes long.
        unsafe {
            core::ptr::copy_nonoverlapping(
                buf.as_ptr().add(start).cast::<u8>(),
                out.as_mut_ptr().add(is_neg as usize),
                end - start,
            );
        }
        // SAFETY: ASCII digits, '.' and '-' only.
        Some(unsafe { str::from_utf8_unchecked(out) })
    }

    fn checked_from_int(self, scale: i32) -> Result<Self, ParseError> {
        // Zero is representable at every scale. Without this, `0` is rejected whenever
        // `10^scale` is not representable in the type (`Overflow` for a large
        // positive scale, `Precision` for a large negative one), even though the
        // string and float conversions accept it.
        if self.is_zero() {
            return Ok(self);
        }

        if scale > 0 {
            let exp = Self::get_exp(scale as usize).ok_or(ParseError::Overflow)?;
            self.checked_mul(&exp).ok_or(ParseError::Overflow)
        } else if scale < 0 {
            let exp = Self::get_exp(scale.unsigned_abs() as usize).ok_or(ParseError::Precision)?;
            if !(self % exp).is_zero() {
                return Err(ParseError::Precision);
            }
            Ok(self / exp)
        } else {
            Ok(self)
        }
    }
}

// Render the non-negative `uns` at `scale` into `buf`, which is filled from the right.
// The caller should handle the sign.
//
// Returns the range `(start, end)` of the rendered text; `end` is `buf.len()` except when
// trailing fraction zeros were dropped.
fn display_num<I>(
    uns: I,
    scale: i32,
    precision: Option<usize>,
    buf: &mut [MaybeUninit<u8>],
) -> (usize, usize)
where
    I: FpdecInner,
{
    let end = buf.len();

    if scale <= 0 {
        let mut offset = buf.len();

        // padding 0 for precision
        let precision = precision.unwrap_or(0);
        if precision != 0 {
            // (a sum could wrap for a huge precision)
            assert!(precision <= 200_usize.saturating_sub(scale.unsigned_abs() as usize));

            offset = pad_zeros(precision, buf);

            // point '.'
            offset -= 1;
            buf[offset].write(b'.');
        }

        // padding 0 for negative scale
        if scale < 0 && !uns.is_zero() {
            offset = pad_zeros(scale.unsigned_abs() as usize, &mut buf[..offset]);
        }

        return (dump_single(uns, &mut buf[..offset]), end);
    }

    // now, scale > 0
    let scale = scale as usize;

    // calculate integer and fraction parts
    let (int, frac, exp) = match I::get_exp(scale) {
        Some(exp) => (uns / exp, uns % exp, Some(exp)),
        None => (I::ZERO, uns, None),
    };

    match precision {
        // no precition set, remove fraction tailing zeros
        None => {
            if frac.is_zero() {
                return (dump_single(int, buf), end);
            }

            // Render all `scale` fraction digits, then drop the trailing zeros. Looking at the
            // bytes is far cheaper than dividing the number by ten once per zero, and `frac`
            // is not zero, so a non-zero digit stops the loop before the point.
            let start = dump_decimal(int, frac, scale, buf);
            let mut end = end;
            // SAFETY: the bytes of `buf[start..]` were written by `dump_decimal`.
            while unsafe { buf[end - 1].assume_init() } == b'0' {
                end -= 1;
            }
            (start, end)
        }

        // set precision = 0, do not show the '.' char
        Some(0) => (
            match exp {
                Some(exp) => {
                    if frac.saturating_add(frac) >= exp {
                        dump_single(int + I::ONE, buf)
                    } else {
                        dump_single(int, buf)
                    }
                }
                // `10^scale` does not fit in `I`, so the value is below 1 -- but not
                // necessarily below 0.5 (`u16` 65535 at scale 5 is 0.65535).
                None => {
                    let rounded = if frac.rounds_to_unit_beyond_range(scale, Rounding::Round) {
                        I::ONE
                    } else {
                        I::ZERO
                    };
                    dump_single(rounded, buf)
                }
            },
            end,
        ),

        // set precision > 0
        Some(precision) => (
            if precision == scale {
                dump_decimal(int, frac, scale, buf)
            } else if precision > scale {
                assert!(precision <= 200);
                let offset = pad_zeros(precision - scale, buf);
                dump_decimal(int, frac, scale, &mut buf[..offset])
            } else {
                let frac = match I::get_exp(scale - precision) {
                    Some(exp) => {
                        let frac = frac.rounding_div(exp, Rounding::Round).unwrap();

                        if let Some(prec_exp) = I::get_exp(precision) {
                            // fraction digits adds 1 more.
                            // for example: 12.99967 with precision=3, the fraction 99967
                            // is round to 1000 whose digits is more than 3.
                            if frac == prec_exp {
                                return (dump_decimal(int + I::ONE, I::ZERO, precision, buf), end);
                            }
                        }

                        frac
                    }
                    // `10^(scale - precision)` does not fit in `I`: the fraction is
                    // below one unit of the last printed place, but may round up to it.
                    None => {
                        if frac.rounds_to_unit_beyond_range(scale - precision, Rounding::Round) {
                            I::ONE
                        } else {
                            I::ZERO
                        }
                    }
                };
                dump_decimal(int, frac, precision, buf)
            },
            end,
        ),
    }
}

// dump: "int . frac"
fn dump_decimal<I>(int: I, frac: I, scale: usize, buf: &mut [MaybeUninit<u8>]) -> usize
where
    I: FpdecInner,
{
    let mut offset = dump_single(frac, buf);

    offset = pad_zeros(scale - (buf.len() - offset), &mut buf[..offset]);

    offset -= 1;
    buf[offset].write(b'.');

    dump_single(int, &mut buf[..offset])
}

// dump a single integer number
fn dump_single<I>(n: I, buf: &mut [MaybeUninit<u8>]) -> usize
where
    I: FpdecInner,
{
    // `size_of` is a constant, so this is not a branch at run time.
    if core::mem::size_of::<I>() <= 8 {
        dump_u64(n.to_u64().unwrap_or(0), buf) // always `Some`: it fits
    } else {
        dump_u128(n.to_u128().unwrap_or(0), buf)
    }
}

// Format per two digits from the lookup table. This is much faster than using integers' Display.
static DECIMAL_PAIRS: &[u8; 200] = b"\
    0001020304050607080910111213141516171819\
    2021222324252627282930313233343536373839\
    4041424344454647484950515253545556575859\
    6061626364656667686970717273747576777879\
    8081828384858687888990919293949596979899";

fn dump_u64(mut n: u64, buf: &mut [MaybeUninit<u8>]) -> usize {
    let mut offset = buf.len();

    while n >= 100 {
        offset -= 2;
        let pair = (n % 100) as usize * 2;
        n /= 100;
        buf[offset].write(DECIMAL_PAIRS[pair]);
        buf[offset + 1].write(DECIMAL_PAIRS[pair + 1]);
    }

    // the last one or two digits; `0` is written as "0"
    if n >= 10 {
        offset -= 2;
        buf[offset].write(DECIMAL_PAIRS[n as usize * 2]);
        buf[offset + 1].write(DECIMAL_PAIRS[n as usize * 2 + 1]);
    } else {
        offset -= 1;
        buf[offset].write(b'0' + n as u8);
    }

    offset
}

// A `u128` division is a slow library call per digit pair, so cut the number into chunks of
// 19 digits (which fit in `u64`) with at most two divisions by a power of ten.
fn dump_u128(n: u128, buf: &mut [MaybeUninit<u8>]) -> usize {
    const E19: u128 = 10_u128.pow(19);

    let Some(high) = div_pow10::bit128::div_single(n, 19) else {
        unreachable!() // 19 <= 38
    };
    if high == 0 {
        return dump_u64(n as u64, buf);
    }
    let low = (n - high * E19) as u64;

    let mut offset = dump_u64(low, buf);
    offset = pad_zeros(19 - (buf.len() - offset), &mut buf[..offset]);

    // `high` is below 3.5e19, and `u64` goes up to 1.8e19
    if let Ok(high) = u64::try_from(high) {
        dump_u64(high, &mut buf[..offset])
    } else {
        let top = high / E19;
        let mid = (high - top * E19) as u64;
        let written_to = offset;
        offset = dump_u64(mid, &mut buf[..written_to]);
        offset = pad_zeros(19 - (written_to - offset), &mut buf[..offset]);
        dump_u64(top as u64, &mut buf[..offset])
    }
}

fn pad_zeros(n: usize, buf: &mut [MaybeUninit<u8>]) -> usize {
    let mut offset = buf.len();
    for _ in 0..n {
        offset -= 1;
        buf[offset].write(b'0');
    }
    offset
}

/// `a * b` at scale difference `diff`, where `b` may be of a DIFFERENT inner type than `a`.
///
/// The usual case (`b` fits `I`) is the plain call. Otherwise -- `I::from(b)?` used to make this
/// `None`, so `0.0001 * 1000.00` (an `i16` times an `i64` of 100000) "overflowed" although the
/// result, 0.1, fits -- the magnitudes are multiplied as `u128`, the sign applied, and the
/// result narrowed to `I`.
#[inline(always)]
pub(crate) fn mul_ext_mixed<I: FpdecInner, J: FpdecInner>(a: I, b: J, diff: i32, rounding: Rounding) -> Option<I> {
    match I::from(b) {
        Some(b) => a.checked_mul_ext(b, diff, rounding),
        None => via_magnitudes(a, b, diff, rounding, <u128 as FpdecInner>::checked_mul_ext),
    }
}

/// `a / b` at scale difference `diff`, where `b` may be of a different inner type than `a`
/// (see [`mul_ext_mixed`]): `0.0001 / 1000.00` is `0` in the result's scale, not `None`.
#[inline(always)]
pub(crate) fn div_ext_mixed<I: FpdecInner, J: FpdecInner>(a: I, b: J, diff: i32, rounding: Rounding) -> Option<I> {
    match I::from(b) {
        Some(b) => a.checked_div_ext(b, diff, rounding),
        None => via_magnitudes(a, b, diff, rounding, <u128 as FpdecInner>::checked_div_ext),
    }
}

/// `op` on the magnitudes of `a` and `b` (every primitive integer's magnitude fits `u128`),
/// with the sign of the result applied afterwards, narrowed to `I`. `None` if it does not fit,
/// or `diff` is out of `I`'s range (`op` would allow `u128`'s).
fn via_magnitudes<I: FpdecInner, J: FpdecInner>(
    a: I,
    b: J,
    diff: i32,
    rounding: Rounding,
    op: fn(u128, u128, i32, Rounding) -> Option<u128>,
) -> Option<I> {
    if diff.unsigned_abs() > I::DIGITS {
        return None;
    }
    let negative = (a < I::ZERO) != (b < J::ZERO);
    // `Floor` / `Ceiling` are about the signed value; the magnitude is rounded the same way
    // as the value when it is positive and the opposite way when it is negative.
    let rounding = match (rounding, negative) {
        (Rounding::Floor, false) | (Rounding::Ceiling, true) => Rounding::TowardsZero,
        (Rounding::Ceiling, false) | (Rounding::Floor, true) => Rounding::AwayFromZero,
        (r, _) => r,
    };
    let (ua, ub) = (num_traits::NumCast::from(a.unsigned_abs())?, num_traits::NumCast::from(b.unsigned_abs())?);
    let m = op(ua, ub, diff, rounding)?;
    if !negative {
        return I::from(m);
    }
    if m == 0 {
        return Some(I::ZERO);
    }
    let v = if m <= i128::MAX as u128 {
        -(m as i128)
    } else if m == 1 << 127 {
        i128::MIN
    } else {
        return None;
    };
    I::from(v)
}
