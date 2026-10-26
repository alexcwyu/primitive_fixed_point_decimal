# Unreleased

- **Faster `i128`/`u128` decimal multiplication and division (12 dp in particular), semantics unchanged.** Signed `calc_mul_div` now works on magnitudes through the `u128` implementation and flips `Floor`/`Ceiling` for negative results, instead of a separate signed double-word path. `calc_mul_div_exp` picks its numerator in one place (`scaled_numerator`): a 63-bit-operand product, a checked 64x128-bit product for 12 dp where division by `10^12` is native, or the previous leading-zeros test; anything that may not fit still takes the double-word division. `TowardsZero` division returns right after the checked quotient, and the 12 dp mul/div dispatch is kept constant for runtime `OobScaleFpdec` scales. Details and measurements: `docs/i128-d96-parity.html`.

- Tests: `tests/arith_adversarial.rs` compares `checked_mul_ext` / `checked_div_ext` (every scale difference in `[-DIGITS, DIGITS]`, all rounding modes, same-width and mixed-width operands, const and out-of-band scales), `calc_mul_div`, `rounding_div`, integer division, ratio multiplication, round-to-multiple, `%`, `+`, `-`, and `round_diff_ext` with a `BigInt` specification. Operands are steered at 2^63, 2^64, 2^126, 2^127, 2^128, powers of ten, quotients at and one past the type limits, and exact ties; mutants of the new fast paths, of the negative-result rounding flip, and of the `TowardsZero` shortcut fail it. Scale with `PFPD_ADV_SCALE`, vary with `PFPD_ADV_SEED`. `tests/i128_fast_path.rs` covers the opaque runtime-scale dispatch.

- Tests: the `u64` `calc_mul_div_exp` rounding-extra overflow regression now uses operands that actually enter the fast path (`u32::MAX * i32::MAX` at 19 digits); the previous `3e9 * 3e9` has 64 leading zeros, one short of the guard, so it never exercised it.

- Tooling: `tests/tools/run_i128_parity.py` (matched i128 mul/div comparison against fixdec D96 at 8/10/12 dp) and `tests/tools/decimal_comparison`.

- Docs: clarify overflow-check-dependent `Neg` behavior and the separate `i64`/`u64` limits of default `FromPrimitive` forwarding.

- Fix: converting a large `u128` decimal to `f32` no longer overflows the mantissa before applying a positive scale; finite scaled values and underflow-to-zero results are preserved.

- Docs: using decimal `*`/`/` operators instead of checked methods also requires a supported scale difference, not just a fitting result and non-zero divisor; doctests show that exact zero results still panic for unsupported scales.

- Tests: const-scale multiplication/division reject scale differences of `+-2^32` rather than wrapping them back to zero; replacing checked narrowing of the three-scale difference with an `as i32` cast previously survived the complete release suite.

- Fix: float scaling uses plain IEEE exponentiation rather than the profile-dependent `powi` intrinsic, so huge-scale conversions now have identical debug and release mantissas (including subnormal `f64` input at scale 320).

- Tests: large-scale float conversions exercise successful chunked scaling near rounding boundaries; changing the `f32`/`f64` chunk sizes by one changes representable mantissas and previously survived the release suite. Fixtures are checked against exact binary-rational values for both signs and both decimal wrappers.

- Tests: `u128` `calc_mul_div` / `checked_mul_ratio_ext` where the first normalised dividend of the double-word reduction equals the divisor exactly (the divisor is the top 128 bits of `|a * b|`, e.g. `(2^90 - 1) * (2^90 + 1) / (2^128 - 1) = 2^52`); the mutant `dividend <= divisor` in `reduce2_big` returned `2^51` there and passed the whole `cargo test --release` suite, even at `PFPD_ORACLE_SCALE=40`.

- Tests: `i128` `calc_mul_div` / `checked_mul_ratio_ext` with `c = +-1` and an exact quotient of `-(2^127 + 1)` (one beyond `MIN`, reached through the final-division branch where the partial quotient is odd) must be `None`; the mutant `q > i128::MAX as u128 + 2` returned `Some(i128::MAX)` and passed the whole `cargo test --release` suite.

- Tests: `TryFrom<integer>` / `fpdec!(int)` is checked against a BigInt specification for all ten source and all ten inner types at scales around 0 and 1 (const and out-of-band), because `S > 0` mutated to `S > 1` (which turns `ConstScaleFpdec<i32, 1>::try_from(100_i8)` into `Overflow`) passed the whole suite.

- Docs: the `None` conditions of the arithmetic methods were listed incompletely. `checked_mul_ratio[_ext]` also return `None` for a zero `b`, and `OobScaleFpdec::checked_mul_const_scale[_ext]` / `checked_div_const_scale[_ext]` also return `None` when the scale `S` of `rhs` is outside `[-DIGITS, DIGITS]` (even for a zero operand); doctests demonstrate each case. `Signed::signum` (and `One::one`) panic for a non-zero value when `1` is not representable at the scale, and `FpdecInner::as_wider` / `from_wider` panic for `i128` / `u128`: documented, with `should_panic` doctests.

- Docs: distinguish exact integer `FromPrimitive` conversions from rounded floating-point conversions; doctests cover positive/negative ties, infinity, and coarse-scale integers.

- Docs: correct the `MAX`/`MIN` mantissa limits used by arithmetic and saturation: for a `b`-bit inner type, signed `MAX` is `2^(b - 1) - 1`, signed `MIN` is `-2^(b - 1)`, and unsigned `MIN` is zero. Doctests verify the formulas for both wrappers.

- **Faster decimal division without LTO:** inline the scale/mixed-type dispatchers so constant scales and rounding reach the integer division; i32 division (including random signs) now matches the widened-i64 reference, and i64/u64 division benefits too. Signed/full-width i32 performance regressions are checked by the arithmetic benchmark tool.

- Tests: every method without `_ext` (`checked_mul`, `checked_div`, `checked_rescale`, `round`, `round_diff`, `checked_round_to_multiple`, `checked_mul_ratio`, `checked_div_int`, `*`, `/`, `*=`, `/=`, ...) is checked to round with `Rounding::Round`, for both wrappers; replacing the default mode of any of 26 forwarding calls by `Floor`/`Ceiling`/`TowardsZero`/`AwayFromZero` previously passed the whole suite.

- Tests: 128-bit `calc_mul_div` / `checked_mul_ratio_ext` when `a * b` overflows -- the sign of the quotient when `a ^ b ^ c == 0`, quotients on and just beyond `MIN`/`MAX`, and a product whose high word reaches the divisor (`mhigh >= c` in `reduce2_big`); mutants of those checks, such as `(self ^ b ^ c) <= 0` and deleting the `return None`, previously survived.

- Tests: a mixed-width result of exactly `MIN + 1` (`-5 * (2 * MAX) / 10` against a `u128` right-hand side) survives the magnitude-to-sign conversion (`m <= i128::MAX as u128` mutated to `<` survived).

- Tests: the `num-traits` `Zero::is_zero` trait method is exercised (`x.is_zero()` resolves to the inherent method, so the trait impl was never called and `==` mutated to `!=` survived).

- Comment correction: `i64`, as well as `u16`, can round to one unit when dropping more digits than its representable power-of-ten range.

- **Faster scale increases:** only special-case zero when the power of ten is out of range; ordinary rescaling can use checked multiplication directly, without an extra branch.

- **Faster `i32` decimal multiplication:** round the exact `i64` product via its magnitude and a rounding extra before dividing, avoiding signed quotient/remainder work (about 2.7x for a scale difference of 4).

- Tests: exercise arithmetic operators, integer/decimal assignments, and owned/reference `Sum` for both wrappers and all ten inner types; an `OobScaleFpdec` `/=`-as-`*=` mutant previously survived.

- Tests: cover negative const target scales and full-width `i32::MIN`/`MAX` rescale differences for every inner type and rounding mode; the missing negative-target branch allowed an arithmetic mutant to survive.

- Tests: guard exact signed `MIN` results from an unsigned mixed-type RHS, including the `u128` magnitude `2^127` conversion branch that survived arithmetic mutation testing.

- Docs: `OobScaleFpdec::checked_mul_ext` drops digits (and can round) for positive, not negative, scale differences; a doctest now demonstrates both signs.

- **Faster `i128` rescaling without LTO:** keep the 64-bit division fast path inline so constant divisors fold through the rescale wrappers (about 3.5x for two dropped digits).

- **Fix: direct `FpdecInner::calc_mul_div_exp` calls with exponent zero** now return the checked product for 64/128-bit types, instead of invoking division routines with invalid unsafe preconditions (debug panic / wrong release result).

- **Faster string parsing (~1.7-2x for typical prices).** `FromStr` and
  `try_from_str` first try a single pass over the bytes for the common shape
  `[+-]digits[.digits]`; everything else, and every error, goes to the previous
  implementation unchanged, so error kinds are identical. A test compares the
  two paths on every string over a small alphabet and at the digit limits.

- **Faster `i64`/`u64` division, `mul_ratio` and `checked_div_ext` with a negative
  scale difference (~2.5x for `i64` `P / P`).** They went through a signed
  `i128` division, which is two library calls plus sign fix-ups; they are now
  one `u128 / u64` on the magnitudes.

- **Faster `u64`/`u128` multiplication and 128-bit integer division without LTO
  (~1.2-1.5x).** `calc_mul_div_exp` of `u64`/`u128` and `rounding_div` of
  `i128`/`u128` lacked `#[inline]`, so a downstream crate built without LTO paid
  a call for each and could not fold the constant scale.

- **Faster `Display` for 128-bit types (~2x).** The digits are cut into 19-digit
  chunks with one division by a power of ten instead of one `u128` division per
  digit pair, and trailing fraction zeros are dropped by looking at the bytes
  rather than dividing by ten once per zero.

- **Fix: `Display` with a scale of `i32::MIN` panicked with an index error**
  (`scale.abs()` overflows; in release builds it wrapped negative and passed the
  "scale <= 200" check). It reports the documented limit now.

- **Add `from_ascii` / `try_from_ascii`**: parse `&[u8]` directly (market data
  arrives as bytes). Non-UTF-8 input is `ParseError::Invalid`.

- **Add `format_into`**: format into a caller's `&mut [u8]` and get the `&str`
  back, without `core::fmt` (~1.5x faster than `Display` for `i64`) and without
  allocating. Same text as `Display` (`{}` / `{:.N}`); `None` if the buffer is
  too short, or where `Display` would panic (scale or precision beyond 200).

- **Add `checked_rescale` / `checked_rescale_ext`** (`ConstScaleFpdec`: target
  scale as a const generic; `OobScaleFpdec`: `diff_scale`): the same number at
  another scale, with the rounding of your choice for the dropped digits.

- **Add `checked_round_to_multiple` / `checked_round_to_multiple_ext`**: round to
  a multiple of a step, e.g. a price to the tick size. The step's sign does not
  matter, including `MIN`.

- **Add `saturating_add`, `saturating_sub`** and, for signed types,
  **`checked_neg`** (`Neg` panics on `MIN` when overflow checks are enabled, and wraps to `MIN` otherwise).

- **Tests: the in-crate unit tests of `calc_mul_div`/`calc_mul_div_exp`, number
  formatting and `rounding_div` are removed.** They compared two implementations
  by the same author on shared operands, or hand-picked tables, and are strictly
  weaker than `tests/oracle.rs` and `tests/parse_format.rs` (arbitrary-precision
  oracle, all ten types); mutating the code under them is caught by the oracle
  alone. The oracle gained targeted inputs it was missing -- results that land
  exactly on `MIN`/`MAX` and `a * n / n` -- which kill two mutants of the 128-bit
  overflow checks that the old tests also missed. `cargo test` of the library
  itself went from ~36 s to instant.

- **Add `num-traits-impls` feature**: `Zero`, `One`, `Num`, `Bounded`, `Signed`,
  `CheckedAdd`/`Sub`/`Mul`/`Div`/`Rem`/`Neg`, `ToPrimitive`, `FromPrimitive`.
  `NumCast`/`PrimInt` are deliberately not implemented -- a decimal is not an
  integer, and `NumCast::from` would have to silently choose a rounding mode.

- **Add `zerocopy` feature**: `IntoBytes`, `FromBytes`, `Immutable`,
  `KnownLayout` for `ConstScaleFpdec` and `OobScaleFpdec`.

- **Add `pyo3` feature**: `IntoPyObject`/`FromPyObject` for `ConstScaleFpdec`
  and `OobFmt`, converting to and from Python's `decimal.Decimal` without loss.
  Accepts `Decimal`, `str` and `int`; rejects `float` and `bool` (the latter is
  an `int` subclass in Python). Implies the new `std` feature.

- **Add `std` and `full` features.** The crate is still `no_std` by default;
  `std` exists because PyO3 requires it.

- **Fix: serde was lossy and broken on binary formats.** `Serialize` wrote an
  `f64`, which holds ~15.9 significant digits where `ConstScaleFpdec<i128, 12>`
  holds up to 39 -- so 123456789.123456789012 came back as 123456789.12345679.
  `Deserialize` called `deserialize_any`, which non-self-describing formats
  reject outright, so bincode and friends failed with
  `DeserializeAnyNotSupported`. Binary formats now write and read the raw
  mantissa: exact, 16 bytes for an `i128`, with the scale carried by the type.
  Human-readable formats are unchanged (exact decimal string).

  **This changes the binary wire format** (`f64` -> mantissa). Anything that
  round-tripped through a *self-describing* binary format such as CBOR or
  MessagePack must be re-encoded; non-self-describing formats could not
  round-trip at all before, so nothing working is broken.

  `Serialize` now requires `I: Serialize` and `Deserialize` requires
  `I: Deserialize`. Every primitive integer satisfies both.

- **Add `ConstScaleFpdec::checked_rem` and the `%` operator.** Required by
  `num_traits::NumOps`, and useful on its own; both operands share a scale, so
  the remainder is exact.

- **Change: the integer `Mul`/`Div`/`MulAssign`/`DivAssign` impls now bound
  `J: Into<I> + PrimInt` instead of `J: Into<I> + Num`.** `Num` was only ever a
  disjointness marker ("the `Num` to avoid conflicting implementations only"),
  and it worked because nothing implemented `Num` for a decimal --
  `num-traits-impls` now does. `PrimInt` is already this crate's marker for
  "primitive integer" (see `IntoRatioInt`) and states the intent directly. All
  primitive integers still work; a custom `Num` type that was also `Into<I>`
  would no longer.

- **Fix: `u64` multiplication with a scale difference of 19 gave wrong results.**
  `calc_mul_div_exp` took its fast path whenever `a * b < 2^64`, then added the
  rounding extra `10^19 - 1` in `u64`, which can overflow: a debug-build panic,
  and in release a wrapped (wrong) quotient -- `3_000_000_000 * 3_000_000_000`
  at diff 19 with `Ceiling` returned `0` instead of `1`. The fast path now
  requires `a * b < 2^63`, and checks the add where it can still overflow (only
  at a scale difference of 19). The check is keyed on the scale difference, a
  constant for `ConstScaleFpdec`, so it is free: a first version that tested the
  sum itself made `u64` multiplication ~10% slower.

- **Fix: `i128`/`u128` multiplication at a scale difference of 38 violated the
  precondition of `div-pow10`'s `unchecked_div_single_r1b`** (`n <= 2^127`; a
  `debug_assert!`, and undefined for an `unsafe` function) when
  `a * b + rounding extra` exceeded it. Debug builds panicked. The other
  division routine is now used for those dividends -- chosen by the scale
  difference (below 38 the dividend cannot reach `2^127`), not by testing the
  dividend, which cost ~40% of an `i128` multiplication (2.8 -> 4.0 ns).

- **Fix: `round`, `round_ext`, `round_diff` and `round_diff_ext` returned a wrong
  value or wrapped silently when the rounded result does not fit.** E.g. rounding
  `1_i8` up to a multiple of 1000 returned `0`; `u16` 65535 at diff 5 (0.65535)
  returned `0`, not "one unit"; larger values wrapped in release. They now panic
  (`"overflow in decimal rounding"`; documented under `# Panics`).
  **Add `checked_round`, `checked_round_ext`, `checked_round_diff` and
  `checked_round_diff_ext`**, returning `None` instead.

- **Fix: `checked_rem` panicked for `MIN % -1`** (it is `0`). Also through
  `num_traits::CheckedRem`.

- **Fix: string parsing.**
  - A sign was accepted inside the fraction: `"1.-5"` parsed as `0.95`, `"1.+5"`
    as `1.05`, `"0.-5"` as `-0.05`. All are now `ParseError::Invalid`.
  - `"00"` (any run of zeros) was `ParseError::Empty`; every spelling of zero
    (`"0"`, `"-0"`, `"+00"`, `"0.000"`) now parses to zero for every inner type,
    including `"-0"` for the unsigned ones.
  - A number too big to hold its own digits (`"1000"` as `i8`, `"1.001"` at scale 3)
    was reported as `Precision`; it is `Overflow` unless it also has more fraction
    digits than the scale (then still `Precision`, as documented).

- **Fix: `Display` with a precision rounded wrongly when `10^scale` does not fit the
  type** (`u16` 65535 at scale 5 with `{:.0}` printed `0` for 0.65535; now `1`).

- **Fix: scale arithmetic overflowed `i32`** for scales near the type's limits:
  `round_diff(i32::MIN)`, `OobFmt::rescale`, `try_from_str(_, i32::MAX)` and the
  `(float, scale)` conversions panicked in debug builds and misbehaved in release.
  They now return the right error / value.

- **Fix: zero was rejected by integer conversion and `OobFmt::rescale` whenever
  `10^scale` is not representable** in the type (`ConstScaleFpdec<i8, 3>::try_from(0)`
  was `Overflow`, `ConstScaleFpdec<i8, -3>::try_from(0)` was `Precision`).

- **Fix (`num-traits-impls`): `ToPrimitive` returned `None` for representable values**
  -- `ConstScaleFpdec<i8, -3>` of value 1000 (it multiplied in the inner type), and
  anything below 1 when `10^S` does not fit (`u16` at scale 5). The integer methods now
  compute in `i128`/`u128`. `FromPrimitive::from_i128`/`from_u128` went through
  `i64`/`u64` and rejected inputs outside those source widths, even when a
  128-bit decimal could represent them.

- **Fix: serde reported "decimal overflow" for every failed integer/float input**,
  including precision loss at a negative scale; it now reports the actual `ParseError`.

- **Fix: `fpdec!` expanded to a hard-coded `primitive_fixed_point_decimal::` path**;
  it uses `$crate`, so it works when the crate is renamed or re-exported.

- Fix: dividing an `OobScaleFpdec` by a `ConstScaleFpdec` panicked with the message
  "overflow in decimal multiplication".

- **Fix: float conversions failed when `10^scale` does not fit the float** (`f32`: scale >= 39,
  `f64`: scale >= 309): `f * inf` is `inf` (or NaN for `0.0`), so even zero was `Overflow`, and
  decimal -> float divided by infinity and returned `0`. The power of ten is now applied in
  chunks.

- **Fix: `ConstScaleFpdec` `checked_mul`/`checked_div` computed the scale difference `S + S2 - SR`
  in `i32`**, which overflows for scales near the limits (panic in debug, wrong value in release);
  it is computed in `i64` and is `None` when out of range.

- Docs: `Rounding::Round` ties go away from zero (it always did).

- **Fix: `checked_mul*` / `checked_div*` with an `rhs` of a different inner type returned `None`
  when `rhs` did not fit the result's type**, even if the result did (`0.0001 * 1000.00` as an
  `i16` times an `i64` holding 100000 is `0.1`). The product is now exact for every pair of inner
  types; the same-type path is unchanged.

- **Fix (`pyo3`): a hostile `Decimal` could force a gigabyte allocation** --
  `Decimal("1E+999999999")` is formatted in full (`format(x, "f")`) before the range check, and
  `Decimal(10**1_000_000)` takes seconds. The exponent / bit length is checked first; such values
  are a `ValueError`.

- Docs: the operators (`+`, `-`, `abs`, ...) panic on overflow in debug builds and wrap in release,
  like the primitive integers; `checked_*` is the profile-independent form.

- **Tests.** An arbitrary-precision oracle (`tests/common`, built on `num-bigint`,
  written from the definition of each rounding mode and sharing no code with the
  crate) now checks every arithmetic, rounding, parsing, formatting and conversion
  entry point for all ten inner types, with boundary-value generation (powers of
  ten and two and their neighbours, exact ties, `MIN`/`MAX`, products straddling
  word boundaries); the 8-bit types are exhaustive. Run with `cargo test` **and**
  `cargo test --release` (debug builds panic on overflow, release builds wrap), and
  scale the sampling with `PFPD_ORACLE_SCALE=10`.

# v1.5.0 (2026-08-08)

- Use div-pow10 crate.
- Fix rounding bug for negative quotient.
- Rewrite fuzz.

# v1.4.2 (2026-04-18)

- Fix bug in division for i128 type.

# v1.4.1 (2026-04-17)

- Optimize multiplication of 64-bit and 128-bit for small numbers.
- Fix Display if the fraction part rounding up to exceed exp.
- Add fuzz tests.

# v1.4.0 (2026-03-14)

- Optimize multiplication of 64-bit and 128-bit.

# v1.3.1 (2026-03-05)

- Update the module document only.

# v1.3.0 (2026-02-05)

- Implement `core::iter::Sum`.
- Fix and optimize `rounding_div()`.
- Add benchmark.

# v1.2.1 (2026-01-29)

- Optimize Display.

# v1.2.0 (2026-01-18)

- Add formatting options supporting, such as width and precision.

# v1.1.0 (2026-01-01)

- Add `OobScaleFpdec::checked_mul_const_scale` and more.

# v1.0.1 (2025-12-31)

- Add `#[must_use]` for many APIs.
- Add `#[repr(transparent)]` for the type.
- Add `TryFrom` for unsigned integers.

# v1.0.0 (2025-12-01)

- Just update to v1.0.0!

# v0.11.0 (2025-10-20)

- Remove the *Cumulative Error*.
- Remove the `CumErr` argument for all `*_ext` methods.
- Rename `round_with_rounding` method to `round_ext`.
- Add CHANGELOG.md.

# v0.10.1 (2025-10-18)

- Add `no-std::no-alloc` category in Cargo.toml.

# v0.10.0 (2025-10-13)

- Add `checked_mul_ratio` method.
- Remove deprecated `to_float` method.

# v0.9.1 (2025-09-22)

- Update README.

# v0.9.0 (2025-09-12)

- Add unsigned support.

# v0.8.1 (2025-07-08)

- Implement `Mul` and `Div` traits.
- Add `to_f32` and `to_f64` to replace `to_float`.
- Fix division without rounding.

# v0.8.0 (2025-06-30)

- Fix division !!!
- Export `FpdecInner` if some one want to implement trait for Decimal.
- Some other optimization.

# v0.7.2 (2025-06-19)

- Add more test.

# v0.7.1 (2025-06-17)

- Fix serde.
 
# v0.7.0 (2025-06-16)

- Rename to `ConstScaleFpdec` and `OobScaleFpdec`.
- Make into `no-std`.
