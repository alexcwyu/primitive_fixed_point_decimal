macro_rules! define_none_scale_common {
    () => {
        /// The zero value.
        pub const ZERO: Self = Self(I::ZERO);

        /// The largest value: (2<sup>b-1</sup> - 1) / 10<sup>S</sup> for signed `I`,
        /// or (2<sup>b</sup> - 1) / 10<sup>S</sup> for unsigned `I`, where `b` is
        /// the number of bits of `I`.
        ///
        /// ```
        /// use primitive_fixed_point_decimal::{ConstScaleFpdec as C, OobScaleFpdec as D};
        /// for m in [C::<i8, 0>::MAX.mantissa(), D::<i8>::MAX.mantissa()] {
        ///     assert_eq!(m as u16, (1_u16 << (i8::BITS - 1)) - 1);
        /// }
        /// for m in [C::<u8, 0>::MAX.mantissa(), D::<u8>::MAX.mantissa()] {
        ///     assert_eq!(m as u16, (1_u16 << u8::BITS) - 1);
        /// }
        /// ```
        pub const MAX: Self = Self(I::MAX);

        /// The smallest value: -2<sup>b-1</sup> / 10<sup>S</sup> for signed `I`,
        /// or zero for unsigned `I`, where `b` is the number of bits of `I`.
        ///
        /// ```
        /// use primitive_fixed_point_decimal::{ConstScaleFpdec as C, OobScaleFpdec as D};
        /// for m in [C::<i8, 0>::MIN.mantissa(), D::<i8>::MIN.mantissa()] {
        ///     assert_eq!(m as i16, -(1_i16 << (i8::BITS - 1)));
        /// }
        /// for m in [C::<u8, 0>::MIN.mantissa(), D::<u8>::MIN.mantissa()] {
        ///     assert_eq!(m, 0);
        /// }
        /// ```
        pub const MIN: Self = Self(I::MIN);

        /// The smallest difference value, 10<sup>-S</sup> .
        pub const EPSILON: Self = Self(I::ONE);

        /// The largest powers of 10.
        pub const MAX_POWERS: Self = Self(I::MAX_POWERS);

        /// Approximate number of significant digits in base 10.
        pub const DIGITS: u32 = I::DIGITS;
        /// Checked addition. Computes `self + rhs`, returning `None` if overflow occurred.
        ///
        /// If you make sure no overflow error, you can use `+` and `+=` instead
        /// for convenience.
        ///
        /// The right operand must have the same scale with self.
        #[must_use]
        pub fn checked_add(self, rhs: Self) -> Option<Self> {
            self.0.checked_add(&rhs.0).map(Self)
        }

        /// Checked subtraction. Computes `self - rhs`, returning `None` if overflow occurred.
        ///
        /// If you make sure no overflow error, you can use `-` and `-=` instead
        /// for convenience.
        ///
        /// The right operand must have the same scale with self.
        #[must_use]
        pub fn checked_sub(self, rhs: Self) -> Option<Self> {
            self.0.checked_sub(&rhs.0).map(Self)
        }

        /// Saturating addition. Computes `self + rhs`, clamping at `MIN` / `MAX` instead of overflowing.
        ///
        /// The right operand must have the same scale with self.
        #[must_use]
        pub fn saturating_add(self, rhs: Self) -> Self {
            Self(num_traits::Saturating::saturating_add(self.0, rhs.0))
        }

        /// Saturating subtraction. Computes `self - rhs`, clamping at `MIN` / `MAX` instead of overflowing.
        ///
        /// The right operand must have the same scale with self.
        #[must_use]
        pub fn saturating_sub(self, rhs: Self) -> Self {
            Self(num_traits::Saturating::saturating_sub(self.0, rhs.0))
        }

        /// Round to a multiple of `step`, such as a price to the tick size.
        ///
        /// Equivalent to [`Self::checked_round_to_multiple_ext`] with `Rounding::Round`.
        #[must_use]
        pub fn checked_round_to_multiple(self, step: Self) -> Option<Self> {
            self.checked_round_to_multiple_ext(step, Rounding::Round)
        }

        /// Round to a multiple of `step`, such as a price to the tick size, with the
        /// rounding you choose.
        ///
        /// `step` has the scale of `self`; its sign does not matter. Returns `None` if
        /// `step` is zero, or the multiple does not fit (rounding `127` of an `i8` up to a
        /// multiple of `10`).
        ///
        /// Examples:
        ///
        /// ```
        /// // Here we use `ConstScaleFpdec` as example. It's same for `OobScaleFpdec`.
        /// use primitive_fixed_point_decimal::{ConstScaleFpdec, Rounding, fpdec};
        /// type Price = ConstScaleFpdec<i64, 4>;
        ///
        /// let tick: Price = fpdec!(0.05);
        /// let bid: Price = fpdec!(10.1234);
        /// assert_eq!(bid.checked_round_to_multiple_ext(tick, Rounding::Floor), Some(fpdec!(10.10)));
        /// assert_eq!(bid.checked_round_to_multiple_ext(tick, Rounding::Ceiling), Some(fpdec!(10.15)));
        /// assert_eq!(bid.checked_round_to_multiple(tick), Some(fpdec!(10.10)));
        /// assert_eq!(bid.checked_round_to_multiple(Price::ZERO), None);
        /// ```
        #[must_use]
        pub fn checked_round_to_multiple_ext(self, step: Self, rounding: Rounding) -> Option<Self> {
            // Every value is a multiple of 1 and of -1, and `MIN / -1` would overflow on the way.
            if step.0 == I::ONE || (I::MIN < I::ZERO && step.0 == I::ZERO - I::ONE) {
                return Some(self);
            }
            // Dividing by a negative step negates the quotient, which swaps what `Floor` and
            // `Ceiling` mean for the multiple. (Negating `step` instead would fail for `MIN`.)
            let rounding = match rounding {
                Rounding::Floor if step.0 < I::ZERO => Rounding::Ceiling,
                Rounding::Ceiling if step.0 < I::ZERO => Rounding::Floor,
                r => r,
            };
            self.0.rounding_div(step.0, rounding)?.checked_mul(&step.0).map(Self)
        }

        /// Checked multiplication with integer. Computes `self * n`, returning
        /// `None` if overflow occurred.
        ///
        /// If you make sure no overflow error, you can use `*` and `*=` instead
        /// for convenience.
        #[must_use]
        pub fn checked_mul_int(self, n: impl Into<I>) -> Option<Self> {
            self.0.checked_mul(&n.into()).map(Self)
        }

        /// Computes `self * a/b`, returning `None` if `b` is zero or overflow occurred.
        ///
        /// Equivalent to [`Self::checked_mul_ratio_ext`] with `Rounding::Round`.
        #[must_use]
        pub fn checked_mul_ratio<R>(self, a: R, b: R) -> Option<Self>
        where
            R: IntoRatioInt<I>,
        {
            self.checked_mul_ratio_ext(a, b, Rounding::Round)
        }

        /// Computes `self * a/b`, returning `None` if `b` is zero or overflow occurred
        /// (the rounded result does not fit).
        ///
        /// The arguments `a` and `b` could be primitive integers,
        /// `ConstScaleFpdec` or `OobScaleFpdec` with same scale.
        ///
        /// Compared to calling [`Self::checked_mul_int`] and [`Self::checked_div_int`]
        /// separately, this can avoid some potential overflow.
        ///
        /// Examples:
        ///
        /// ```
        /// // Here we use `ConstScaleFpdec` as example. It's same for `OobScaleFpdec`.
        /// use primitive_fixed_point_decimal::{ConstScaleFpdec, fpdec};
        /// type Balance = ConstScaleFpdec<i64, 2>;
        /// type Quantity = ConstScaleFpdec<i32, 4>; // type for `a` and `b`
        ///
        /// let margin: Balance = fpdec!(30000);
        /// let deal: Quantity = fpdec!(0.2);
        /// let total: Quantity = fpdec!(0.3);
        /// assert_eq!(margin.checked_mul_ratio(deal, total).unwrap(), fpdec!(20000));
        ///
        /// // integer
        /// assert_eq!(margin.checked_mul_ratio(200, 300).unwrap(), fpdec!(20000));
        ///
        /// // `None` for a zero `b`, and for a result that does not fit
        /// assert_eq!(margin.checked_mul_ratio(200, 0), None);
        /// assert_eq!(Balance::MAX.checked_mul_ratio(2, 1), None);
        /// ```
        #[must_use]
        pub fn checked_mul_ratio_ext<R>(self, a: R, b: R, rounding: Rounding) -> Option<Self>
        where
            R: IntoRatioInt<I>,
        {
            self.0
                .calc_mul_div(a.to_int(), b.to_int(), rounding)
                .map(Self)
        }

        /// Checked division by integer, with `Rounding::Round`.
        ///
        /// Computes `self / n`, returning `None` if `n == 0` or overflow occurres.
        ///
        /// If you make sure no overflow or 0-division error, you can use
        /// `/` and `/=` instead for convenience.
        #[must_use]
        pub fn checked_div_int(self, n: impl Into<I>) -> Option<Self> {
            self.checked_div_int_ext(n, Rounding::Round)
        }

        /// Checked division by integer with rounding.
        ///
        /// Computes `self / n`, returning `None` if `n == 0` or overflow occurres.
        #[must_use]
        pub fn checked_div_int_ext(self, n: impl Into<I>, rounding: Rounding) -> Option<Self> {
            self.0.rounding_div(n.into(), rounding).map(Self)
        }

        /// Return if zero.
        #[must_use]
        pub fn is_zero(&self) -> bool {
            self.0.is_zero()
        }

        /// Create a decimal from the underlying integer representation.
        ///
        /// You must take care of the scale yourself.
        ///
        /// This method is `const`, so it can be used to construct const values.
        ///
        /// Examples:
        ///
        /// ```
        /// // Here we use `ConstScaleFpdec` as example. It's same for `OobScaleFpdec`.
        /// use primitive_fixed_point_decimal::{ConstScaleFpdec, fpdec};
        /// type Dec = ConstScaleFpdec<i32, 4>; // scale is 4
        ///
        /// assert_eq!(Dec::from_mantissa(123400), fpdec!(12.34));
        ///
        /// // const values
        /// const ONE: Dec = Dec::from_mantissa(1 * 10000);
        /// const TEN: Dec = Dec::from_mantissa(10 * 10000);
        /// const PI: Dec = Dec::from_mantissa(31400);
        /// assert_eq!(ONE, fpdec!(1));
        /// assert_eq!(TEN, fpdec!(10));
        /// assert_eq!(PI, fpdec!(3.14));
        ///
        /// // you can use `Dec::SCALE` for better consistency
        /// const ONE_1: Dec = Dec::from_mantissa(10_i32.pow(Dec::SCALE as u32));
        /// const TEN_1: Dec = Dec::from_mantissa(10_i32.pow(Dec::SCALE as u32 + 1));
        /// const PI_1: Dec = Dec::from_mantissa(314 * 10_i32.pow(Dec::SCALE as u32 - 2));
        /// assert_eq!(ONE_1, fpdec!(1));
        /// assert_eq!(TEN_1, fpdec!(10));
        /// assert_eq!(PI_1, fpdec!(3.14));
        /// ```
        #[must_use]
        pub const fn from_mantissa(i: I) -> Self {
            Self(i)
        }

        /// Return the underlying integer representation.
        ///
        /// You must take care of the scale yourself.
        ///
        /// Examples:
        ///
        /// ```
        /// // Here we use `ConstScaleFpdec` as example. It's same for `OobScaleFpdec`.
        /// use primitive_fixed_point_decimal::{ConstScaleFpdec, fpdec};
        /// type Dec = ConstScaleFpdec<i32, 4>; // scale is 4
        ///
        /// let d: Dec = fpdec!(12.34);
        /// assert_eq!(d.mantissa(), 123400);
        /// ```
        #[must_use]
        pub const fn mantissa(self) -> I {
            self.0
        }
    };
}

macro_rules! define_none_scale_common_signed {
    () => {
        /// Computes the absolute value of self.
        ///
        /// # Overflow behavior
        ///
        /// The absolute value of `MIN` cannot be represented as this type,
        /// and attempting to calculate it will cause an overflow. This means that
        /// code in debug mode will trigger a panic on this case and optimized code
        /// will return `MIN` without a panic.
        #[must_use]
        pub fn abs(self) -> Self {
            Self(self.0.abs())
        }

        /// Checked negation. Computes `-self`, returning `None` if `self == MIN`.
        #[must_use]
        pub fn checked_neg(self) -> Option<Self> {
            if self == Self::MIN {
                None
            } else {
                Some(Self(-self.0))
            }
        }

        /// Checked absolute value. Computes `self.abs()`, returning `None` if `self == MIN`.
        #[must_use]
        pub fn checked_abs(self) -> Option<Self> {
            if self == Self::MIN {
                None
            } else {
                Some(self.abs())
            }
        }

        /// Return if negative.
        #[must_use]
        pub fn is_neg(&self) -> bool {
            self.0.is_negative()
        }

        /// Return if positive.
        #[must_use]
        pub fn is_pos(&self) -> bool {
            self.0.is_positive()
        }
    };
}

pub(crate) use define_none_scale_common;
pub(crate) use define_none_scale_common_signed;
