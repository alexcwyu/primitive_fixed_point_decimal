# Arithmetic and merge review

## Integration review, 2026-10-26

Verdict: approved and merged into local `master`. No new high- or medium-severity issue was found in this review. The low-priority performance caveat below remains.

### Synchronization and merge

- Fetched `origin/master` and merged it into `alex-yu/review` before reviewing. It was still at `3f6e82a568dee9f3dde619bc6ac02b7f4448ab58`, already included locally, so no incoming changes or conflicts needed resolution.
- Pulled local `master` again immediately before integration. It was still up to date.
- Committed the six production, benchmark and changelog files as `906bdb9`, then fast-forwarded local `master` to that commit.
- Ran the all-feature debug and release suites again against the merged `master` source. Both passed, 310 tests and doctests each.
- This report is committed separately. Nothing was pushed remotely; the review branch and workspace are retained.

### Review scope and findings

Reviewed the changed dispatchers, both decimal wrappers, 128-bit integer implementations, their callers, and existing boundary/oracle tests. A fresh independent review also found no blocking issue.

The extracted signed and unsigned overflow bodies mechanically match the baseline after renaming `self` to `a`. No arithmetic expression, sign restoration, zero-divisor guard or overflow check changed. The small checked-product branch forwards the original rounding mode. The rounding fix changes inlining boundaries, not arithmetic rules. There is no public API or declared dependency change.

Existing tests cover zero divisors, signed `MIN` and `MAX`, unsigned limits, exact and near-half ties, all five rounding modes, intermediate overflow with a representable result, zero and unsupported exponents, extreme scales, and mixed integer widths/signedness. The benchmark checks complete `Option` results before timing rather than hiding false overflow as zero.

### Fresh verification

| Check | Result |
|---|---|
| `cargo test --all-features` | 310 tests/doctests passed |
| `cargo test --release --all-features` | 310 passed |
| `cargo test` | 271 passed |
| `PFPD_ORACLE_SCALE=20 cargo test --release --all-features --test oracle` | 23 passed with expanded boundary/random sampling |
| `cargo check --no-default-features` | Passed |
| `cargo check --no-default-features --target aarch64-unknown-linux-gnu` | Passed, compile only |
| Benchmark, seven interleaved runs without LTO | 58 comparisons completed, all 15 limits passed |
| Benchmark, seven interleaved runs with fat LTO | 58 comparisons completed, all 15 limits passed |
| All-feature debug and release tests after merging into `master` | 310 passed in each profile |

Two deliberately broken variants were tested in an isolated copy, never in the working tree being merged:

- Replacing the signed fast path's rounding mode with `Floor` caused 11,488 oracle mismatches.
- Removing the unsigned overflow fallback caused 81,692 oracle mismatches.

Both were rejected by `calc_mul_div_128bit_signs_and_quotient_limits`. These checks confirm that existing tests detect wrong rounding and false overflow in the changed dispatch paths.

Fresh decimal timings below are ns/op on the same Intel i7-12800H, pinned to CPU 2. These are current implementation timings, not a fresh baseline speedup comparison.

| Operation | Without LTO | Fat LTO |
|---|---:|---:|
| `i128` decimal multiply | 6.24 | 5.35 |
| `u128` decimal divide | 2.45 | 2.39 |
| `i128` multiply ratio | 4.26 | 4.20 |
| `u128` multiply ratio | 2.83 | 2.84 |
| `i128` precision rounding, mixed signs | 3.23 | 3.32 |
| `i128` runtime precision rounding, mixed signs | 4.39 | 5.07 |

### Remaining caveats

The earlier broad probe found approximately 10% slower full-width constant-scale `i128` rounding with LTO. That specific broad probe was not rerun today; the known tradeoff is not claimed fixed. The fresh benchmark gates cover bounded mantissas, not every full-width workload.

CPU frequency was not locked. ARM execution, long-running fuzzing, and a comprehensive mutation/line-coverage score were not checked. No production code changed during this integration review, so the earlier four-million-case checks and baseline performance tables below remain historical evidence from October 23, not additional fresh runs.

---

## Original arithmetic review, 2026-10-23

2026-10-23. Baseline: `3f6e82a`.

What this repo does: fixed-point decimal arithmetic over ten signed and unsigned integer types, with constant or caller-supplied scales. This review assumes latency-sensitive financial calculations, including small mantissas stored in 128-bit types.

Remote synchronization: `origin` has no `develop` branch. Fetched `feature/ayu_develop` instead, at `6eaa3db`. Merging it with `--ff-only` reported already up to date; the review branch already includes it.

## Findings fixed

No new high- or medium-severity arithmetic correctness defect was reproduced. The first pass addressed three medium-severity performance and benchmark-coverage issues. A second adversarial pass found and fixed a performance regression introduced by the rounding optimization. A smaller remaining performance tradeoff is recorded below.

### 1. Constant scales were lost across multiplication and rounding dispatch

`src/fpdec_inner.rs`: `checked_mul_ext` and `mul_ext_mixed`; `src/const_scale_fpdec.rs`: `checked_round_ext`.

These helpers route decimal arithmetic to the integer implementation. Without inlining, some downstream callers retained helper calls instead of specializing for the known scale and rounding mode. The final before/after benchmark measures scale-4 `u128` multiplication at 7.84 versus 4.43 ns/op.

Fixed by preserving constant scales at the dispatch boundaries, following the existing division and rescale pattern. Rounding needs forced inlining at the public constant-scale wrapper, not at the shared runtime-scale implementation, as the second pass demonstrated. No arithmetic expression or overflow check changed.

### 2. The double-width ratio implementation prevented a cheap single-width path

`src/inner_i128.rs`: both `calc_mul_div` implementations.

These methods calculate a rounded `a * b / c`. Even when the product fits in one integer, the large function containing the double-width fallback prevented effective caller optimization. Small `u128` decimal division took 6.68 ns/op; `i128` and `u128` ratios took 7.61 and 6.69 ns/op.

Fixed by keeping the checked product and division in a small inline method and moving the existing overflow bodies into private functions. The results are 2.21, 3.98, and 2.68 ns/op respectively. The double-width algorithms, sign restoration, zero-divisor handling, and checked final additions are unchanged. A mechanical comparison confirmed that the extracted bodies differ only in indentation and renaming `self` to `a`.

### 3. The benchmark missed these 128-bit arithmetic paths

`tests/tools/bench_arithmetic.rs` and `tests/tools/bench_arithmetic.py`.

The benchmark covered some `i128` operations, but decimal multiplication, division, ratios, integer division, and precision rounding were only measured for `i32`, `i64`, and `u64`. It therefore could not catch the preceding slow paths.

Added both 128-bit types to those measurements and added regression limits for the measured gaps. The benchmark also compares complete `Option` results with its reference before timing, so an incorrect `None` cannot masquerade as zero through `map_or(0)`. The new unsigned multiplication and division limits failed before the fixes.

The second pass added mixed-sign precision rounding, caller-supplied scales with all five rounding modes, and an optional fat-LTO build. The driver uses Cargo's reported library artifact rather than the first matching cached library, so changing build flags cannot silently select the wrong artifact.

### 4. The initial rounding optimization regressed signed and runtime-scale callers

Medium severity, introduced in the first pass and fixed in the second.

Forcing `checked_round_diff_with_rounding` inline made `ConstScaleFpdec::checked_round_ext` too large for normal inlining in some callers. The caller's constant scale was then lost. A separate mixed-sign probe measured constant-scale `i128` rounding without LTO at 3.01 versus 6.04 ns/op. Runtime-scale rounding with LTO went from 4.38 to 7.24 ns/op. The original positive-only benchmark missed both regressions.

The fix changes two attributes: ordinary `#[inline]` on the shared rounding implementation and `#[inline(always)]` on the public constant-scale wrapper. The same broad probes now measure 3.14 ns/op for signed constant-scale rounding and 4.38 ns/op for runtime-scale rounding with LTO. No rounding formula changed.

The expanded benchmark's signed-rounding limit failed against the initial implementation with `--lto` and passed after the fix. Both rounding workloads now have performance limits; they also compare complete results against an independently computed reference with hand-checked sign and tie fixtures.

## Correctness and coverage assessment

Reviewed the arithmetic implementations and their callers in both decimal wrappers, shared helpers, integer specializations, and `num-traits` adapters. The existing coverage is substantial:

| Area | Evidence |
|---|---|
| Add, subtract, integer multiply/divide, remainder | BigInt comparisons, overflow checks, zero divisors, signed limits, operator and assignment tests |
| Decimal multiply/divide | Exhaustive 8-bit operand pairs; sampled boundaries for larger types; all five rounding modes; positive, zero and negative scale differences |
| Mixed widths and signedness | All 100 pairs of supported integer types; representable results despite an oversized RHS; negative results and unsigned zero |
| Ratios and 256-bit intermediate products | BigInt oracle; signed `MIN` restoration; quotient limits; reduction boundaries; final rounded overflow |
| Rescale, precision rounding, tick rounding, saturation | BigInt specifications, negative steps including `MIN`, extreme scales, ties and clamping |
| Public integer helpers | Direct calls including exponent zero and unsupported exponents, not just wrapper calls |

Additional adversarial checks exercised four million 128-bit cases per profile in both debug and release, first on the baseline and again on the final code. These covered every exponent from 0 through 38 and all rounding modes. Ratio denominators included operand neighbors, values near `2^127` and `u128::MAX`, and random bit lengths. No mismatch occurred.

A mutation replacing the signed 128-bit fast path's requested rounding with `Floor` produced 249,447 oracle mismatches. Restoring the implementation returned the suite to green. This confirms that existing tests detect incorrect rounding in the extracted fast path.

The supported scale-difference limits remain intentional. Multiplication and division can return `None` outside those limits even when the mathematical result is zero. This review did not expand that contract.

## Performance measurements

Intel Core i7-12800H, x86-64, Rust 1.99.0, pinned to CPU 2. Seven interleaved baseline/final runs used the same final benchmark source and data. Values below are minimum ns/op without LTO, matching the existing benchmark's method; medians confirmed the main improvements.

| Operation | Before | After | Speedup |
|---|---:|---:|---:|
| `i64` decimal multiply | 2.80 | 1.68 | 1.67x |
| `u64` decimal multiply | 2.34 | 1.19 | 1.97x |
| `i128` decimal multiply | 8.56 | 5.59 | 1.53x |
| `u128` decimal multiply | 7.84 | 4.43 | 1.77x |
| `u128` decimal divide | 6.68 | 2.21 | 3.02x |
| `i128` multiply ratio | 7.61 | 3.98 | 1.91x |
| `u128` multiply ratio | 6.69 | 2.68 | 2.50x |
| `i128` precision rounding, positive | 2.78 | 2.83 | 0.98x |
| `i128` precision rounding, mixed signs | 2.89 | 2.96 | 0.98x |
| `i128` runtime precision rounding, mixed signs | 4.72 | 3.99 | 1.18x |

Multiplication and ratio rows use bounded positive mantissas. Rounding results depend strongly on the surrounding caller code: the initial positive-only benchmark measured 7.61 to 2.79 ns/op, but the expanded final benchmark also lets the baseline optimize that case. That earlier result is not a general rounding speedup.

A separate before/after check included random signed small values, half-width values, full-width values, runtime scales, runtime modes, and mixed integer widths, with and without fat LTO. The earlier full-width ratio comparison was essentially unchanged: signed 511.10 to 512.28 ns/op, unsigned 375.95 to 374.91 ns/op. The small-value speedups must not be read as improvements to the double-width reduction algorithm.

#### Remaining low-priority performance tradeoff

In the broad probe, full-width constant-scale `i128` rounding with LTO measured 7.38 to 8.14 ns/op; medians were 7.44 to 8.23. This roughly 10% slowdown is much smaller than the fixed regressions but is real in this caller. It persists when the rounding-specific attributes are reverted, so reverting that optimization alone does not remove it. No further speculative inlining was added to chase this sub-nanosecond difference. Workloads dominated by full-width LTO rounding should benchmark before adopting the change.

The broad probe's executable text grew by 196 bytes without LTO and shrank by 32 bytes with LTO. These measurements do not guarantee the same code size or timing in other consumers.

Experiment record: adding ordinary or forced inline attributes to the original large `calc_mul_div` functions left ratio calls near 7 ns/op. Those versions were discarded in favor of separating the existing overflow path. No dependency or public API was added.

## Verification

- `cargo test --all-features`: 310 tests and doctests passed.
- `cargo test --release --all-features`: 310 passed.
- `cargo test`: 271 passed with default features.
- `cargo check --no-default-features`, `cargo check --all-features`, and `cargo check --no-default-features --target aarch64-unknown-linux-gnu`: passed.
- `PFPD_ORACLE_SCALE=20 cargo test --release --all-features --test oracle`: 23 passed on the baseline.
- Independent four-million-case 128-bit oracle: passed in both profiles on the final code.
- Two read-only reviews of the arithmetic extraction: no concrete correctness issue found. Performance was measured separately, not by those reviewers.
- `python3 tests/tools/bench_arithmetic.py --runs 7`: all 58 comparisons completed and all 15 regression limits passed.
- The same command with `--lto`: all 58 comparisons completed and all 15 limits passed. Both modes reused the same scratch directory to check artifact selection across changed build flags.
- `git diff --check`: passed.

Reproduce the performance regression checks with:

```sh
python3 tests/tools/bench_arithmetic.py --runs 7
python3 tests/tools/bench_arithmetic.py --runs 7 --lto
```

## Limits

ARM was compile-checked, not executed or benchmarked. No long-running fuzz campaign or external cross-model review was run. CPU frequency was not locked, so timings and regression limits are machine-specific. Parsing, formatting and interoperability were not deeply audited, although their normal tests ran. No line-coverage percentage or comprehensive mutation score was measured.

Verdict: the identified medium issues are fixed. No high-severity issue was found. The full-width LTO rounding tradeoff remains documented; this is not a claim that every arithmetic workload became faster.
