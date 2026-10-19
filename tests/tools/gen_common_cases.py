#!/usr/bin/env python3
"""Generate tests/common_cases.rs (run from the crate root: python3 tests/tools/gen_common_cases.py). realistic trading cases whose expected values come from Python's
`decimal` module (exact, 200 digits), independent of the library AND of the Rust bigint oracle."""
import random
from decimal import Decimal, getcontext, ROUND_HALF_UP, ROUND_FLOOR, ROUND_CEILING, ROUND_DOWN, ROUND_UP

getcontext().prec = 200
random.seed(20261019)
MODES = {"Round": ROUND_HALF_UP, "Floor": ROUND_FLOOR, "Ceiling": ROUND_CEILING,
         "TowardsZero": ROUND_DOWN, "AwayFromZero": ROUND_UP}
RANGE = {"i32": (-2**31, 2**31 - 1), "i64": (-2**63, 2**63 - 1), "i128": (-2**127, 2**127 - 1),
         "u32": (0, 2**32 - 1), "u64": (0, 2**64 - 1)}

def fmt(d, scale):
    q = Decimal(1).scaleb(-scale)
    return format(d.quantize(q), "f")

def fits(ty, d, scale):
    m = d.scaleb(scale)
    lo, hi = RANGE[ty]
    return m == m.to_integral_value() and lo <= m <= hi

def rnd_dec(ty, scale, int_digits, neg_ok=True):
    ip = random.randint(0, 10**int_digits - 1)
    fp = random.randint(0, 10**scale - 1) if scale > 0 else 0
    s = f"{ip}.{fp:0{scale}d}" if scale > 0 else f"{ip}"
    d = Decimal(s)
    if neg_ok and RANGE[ty][0] < 0 and random.random() < 0.3:
        d = -d
    return d

def tie_dec(scale, k):  # a value that makes products/quotients land on exact ties often
    return Decimal(random.choice([5, 15, 25, 35, 125, 375])) .scaleb(-scale + k)

def cases_for(ty, scales, int_digits):
    mul, div, ratio, rnd, rescale, mult = [], [], [], [], [], []
    S = scales
    for _ in range(4):
        for (s1, s2, sr) in [(S[0], S[1], S[2]), (S[0], S[0], S[0]), (S[1], S[1], 0), (S[2], S[0], S[1])]:
            a = rnd_dec(ty, s1, int_digits); b = rnd_dec(ty, s2, max(1, int_digits - 3))
            if random.random() < 0.25:
                a = tie_dec(s1, 0); b = tie_dec(s2, 0)
            if not (fits(ty, a, s1) and fits(ty, b, s2)): continue
            for mode, py in MODES.items():
                if random.random() < 0.5: continue
                exact = a * b
                r = exact.quantize(Decimal(1).scaleb(-sr), rounding=py)
                exp = fmt(r, sr) if fits(ty, r, sr) else None
                mul.append((fmt(a, s1), s1, fmt(b, s2), s2, sr, mode, exp))
                if b != 0:
                    exact = a / b
                    r = exact.quantize(Decimal(1).scaleb(-sr), rounding=py)
                    exp = fmt(r, sr) if fits(ty, r, sr) else None
                    div.append((fmt(a, s1), s1, fmt(b, s2), s2, sr, mode, exp))
    # exact half-way ties, both signs: 1.00 / 8 = 0.125, 0.05 * 0.5 = 0.025, ...
    for sg in (1, -1):
        if sg < 0 and RANGE[ty][0] == 0: continue
        for (s1, s2, sr, bs) in [(S[2], 0, S[2], ["2", "4", "8", "16", "-2"]),       # diff == 0
                                 (S[2], 2, S[2], ["2.00", "4.00", "8.00", "0.50"]),   # diff < 0: mul then divide
                                 (S[0], 0, S[0] - 1, ["2", "4", "5", "10"]),          # diff > 0: divide twice
                                 (S[2], S[2], S[2], ["0.5", "1.5", "2.5", "-0.5"])]:  # tie from a product
            for bstr in bs:
                b = Decimal(bstr)
                if b < 0 and RANGE[ty][0] == 0: continue
                if not fits(ty, b, s2): continue
                for _ in range(1):
                    unit = Decimal(1).scaleb(-sr)
                    n = random.randint(0, 40)
                    # a / b lands on (n + 1/2) units: a = (n + 1/2) * unit * b  (needs a on a's scale)
                    a = (Decimal(n) + Decimal("0.5")) * unit * b * sg
                    if fits(ty, a, s1):
                        for mode, py in MODES.items():
                            r = (a / b).quantize(unit, rounding=py)
                            div.append((fmt(a, s1), s1, fmt(b, s2), s2, sr, mode, fmt(r, sr) if fits(ty, r, sr) else None))
                    # a * b lands on a tie when a is odd * unit/2 ... use a = (2n+1) units of s1 and b = 0.5-like
                    a2 = Decimal(2 * n + 1).scaleb(-s1) * sg
                    if fits(ty, a2, s1) and s1 + s2 - sr <= 0 or True:
                        if fits(ty, a2, s1):
                            for mode, py in MODES.items():
                                r = (a2 * b).quantize(Decimal(1).scaleb(-sr), rounding=py)
                                mul.append((fmt(a2, s1), s1, fmt(b, s2), s2, sr, mode, fmt(r, sr) if fits(ty, r, sr) else None))
    # near-ties: one unit of the last digit either side of half-way, multiplied/divided by 1
    # (the rounding extra of the fast paths is off by one exactly here)
    for sg in (1, -1):
        if sg < 0 and RANGE[ty][0] == 0: continue
        sr = S[2]; s1 = S[0]; unit = Decimal(1).scaleb(-sr)
        for bstr, s2 in (("1.00", 2), ("1", 0)):
            for d in (-1, 1, 0):
                for n in (0, 7, 40):
                    a = ((Decimal(n) + Decimal("0.5")) * unit + d * Decimal(1).scaleb(-s1)) * sg
                    if not fits(ty, a, s1): continue
                    b = Decimal(bstr)
                    for mode, py in MODES.items():
                        r = (a * b).quantize(unit, rounding=py)
                        mul.append((fmt(a, s1), s1, fmt(b, s2), s2, sr, mode, fmt(r, sr) if fits(ty, r, sr) else None))
                        r = (a / b).quantize(unit, rounding=py)
                        div.append((fmt(a, s1), s1, fmt(b, s2), s2, sr, mode, fmt(r, sr) if fits(ty, r, sr) else None))
    # round to scale, rescale, round to multiple
    for _ in range(6):
        s = S[0]; a = rnd_dec(ty, s, int_digits)
        if not fits(ty, a, s): continue
        tick = random.choice(["0.05", "0.25", "0.5", "1", "0.01", "5", "0.0005"])
        tk = Decimal(tick)
        if not fits(ty, tk, s): continue
        for mode, py in MODES.items():
            to = random.randint(0, s)
            r = a.quantize(Decimal(1).scaleb(-to), rounding=py)
            rescale.append((fmt(a, s), s, to, mode, fmt(r, s) if fits(ty, r, s) else None))
            m = (a / tk).to_integral_value(rounding=py) * tk
            mult.append((fmt(a, s), s, tick, mode, fmt(m, s) if fits(ty, m, s) else None))
    return mul, div, rescale, mult

def rs(v):
    return "None" if v is None else f'Some("{v}")'

out = []
out.append('''//! Realistic cases -- prices, quantities, fee rates, notionals, tick sizes -- with expected values
//! computed by Python's `decimal` module (exact, 200 digits). That is a third, independent opinion:
//! `tests/oracle.rs` checks the edges against `num-bigint` written by the same hands as the library,
//! and these are the everyday numbers a trading system multiplies and divides.
//!
//! GENERATED by `tests/tools/gen_common_cases.py` (fixed seed): regenerate rather than edit by hand.
use primitive_fixed_point_decimal::{OobScaleFpdec as D, Rounding, Rounding::*};

type Row = (&'static str, i32, &'static str, i32, i32, Rounding, Option<&'static str>);
type RoundRow = (&'static str, i32, i32, Rounding, Option<&'static str>);
type TickRow = (&'static str, i32, &'static str, Rounding, Option<&'static str>);
''')

groups = [("i64", [8, 4, 2], 6), ("i32", [4, 2, 3], 4), ("u64", [8, 4, 2], 6), ("i128", [18, 8, 4], 12)]
for ty, scales, idg in groups:
    mul, div, rescale, mult = cases_for(ty, scales, idg)
    U = ty.upper()
    out.append(f"const MUL_{U}: &[Row] = &[")
    for r in mul: out.append(f'    ("{r[0]}", {r[1]}, "{r[2]}", {r[3]}, {r[4]}, {r[5]}, {rs(r[6])}),')
    out.append("];")
    out.append(f"const DIV_{U}: &[Row] = &[")
    for r in div: out.append(f'    ("{r[0]}", {r[1]}, "{r[2]}", {r[3]}, {r[4]}, {r[5]}, {rs(r[6])}),')
    out.append("];")
    out.append(f"const RESCALE_{U}: &[RoundRow] = &[")
    for r in rescale: out.append(f'    ("{r[0]}", {r[1]}, {r[2]}, {r[3]}, {rs(r[4])}),')
    out.append("];")
    out.append(f"const TICK_{U}: &[TickRow] = &[")
    for r in mult: out.append(f'    ("{r[0]}", {r[1]}, "{r[2]}", {r[3]}, {rs(r[4])}),')
    out.append("];\n")

out.append('''/// `a (scale s1) * b (scale s2)` at scale `sr`, or the quotient. The table only holds cases whose scale
/// difference is within the type's digits, so every `Some` is exact and every `None` is an overflow.
fn run_rows<T>(name: &str, rows: &[Row], op: &str)
where
    T: primitive_fixed_point_decimal::FpdecInner
        + num_traits::Num<FromStrRadixErr = core::num::ParseIntError>
        + core::fmt::Debug,
{
    let mut bad = Vec::new();
    for &(a, s1, b, s2, sr, mode, want) in rows {
        let x = D::<T>::try_from_str(a, s1).unwrap_or_else(|e| panic!("{name} setup {a} @{s1}: {e}"));
        let y = D::<T>::try_from_str(b, s2).unwrap_or_else(|e| panic!("{name} setup {b} @{s2}: {e}"));
        let diff = if op == "mul" { s1 + s2 - sr } else { s1 - s2 - sr };
        let got = if op == "mul" { x.checked_mul_ext(y, diff, mode) } else { x.checked_div_ext(y, diff, mode) };
        let want_n = want.map(|w| D::<T>::try_from_str(w, sr).unwrap());
        let got_n = got;
        if got_n != want_n {
            bad.push(format!("{name} {op} {a}@{s1} {b}@{s2} -> @{sr} {mode:?}: got {got:?}, want {want:?}"));
        }
    }
    assert!(bad.is_empty(), "{} mismatches:\\n{}", bad.len(), bad.join("\\n"));
}

fn run_rescale<T>(name: &str, rows: &[RoundRow])
where
    T: primitive_fixed_point_decimal::FpdecInner + num_traits::Num<FromStrRadixErr = core::num::ParseIntError> + core::fmt::Debug,
{
    let mut bad = Vec::new();
    for &(a, s, to, mode, want) in rows {
        let x = D::<T>::try_from_str(a, s).unwrap();
        // round to `to` fraction digits, staying at scale `s`
        let got = x.checked_round_diff_ext(s - to, mode);
        let want = want.map(|w| D::<T>::try_from_str(w, s).unwrap());
        if got != want { bad.push(format!("{name} round {a}@{s} to {to} {mode:?}: got {got:?}, want {want:?}")); }
        // the same through rescale: to a smaller scale and back
        if let Some(down) = x.checked_rescale_ext(s - to, mode) {
            let back = down.checked_rescale(to - s);
            if let (Some(back), Some(w)) = (back, want) {
                if back != w { bad.push(format!("{name} rescale {a}@{s} to {to} {mode:?}: back {back:?}, want {w:?}")); }
            }
        }
    }
    assert!(bad.is_empty(), "{} mismatches:\\n{}", bad.len(), bad.join("\\n"));
}

fn run_ticks<T>(name: &str, rows: &[TickRow])
where
    T: primitive_fixed_point_decimal::FpdecInner + num_traits::Num<FromStrRadixErr = core::num::ParseIntError> + core::fmt::Debug,
{
    let mut bad = Vec::new();
    for &(a, s, tick, mode, want) in rows {
        let x = D::<T>::try_from_str(a, s).unwrap();
        let t = D::<T>::try_from_str(tick, s).unwrap();
        let got = x.checked_round_to_multiple_ext(t, mode);
        let want = want.map(|w| D::<T>::try_from_str(w, s).unwrap());
        if got != want { bad.push(format!("{name} tick {a}@{s} / {tick} {mode:?}: got {got:?}, want {want:?}")); }
    }
    assert!(bad.is_empty(), "{} mismatches:\\n{}", bad.len(), bad.join("\\n"));
}
''')
for ty, _, _ in groups:
    U = ty.upper()
    out.append(f'''#[test] fn mul_{ty}() {{ run_rows::<{ty}>("{ty}", MUL_{U}, "mul"); }}
#[test] fn div_{ty}() {{ run_rows::<{ty}>("{ty}", DIV_{U}, "div"); }}
#[test] fn rescale_{ty}() {{ run_rescale::<{ty}>("{ty}", RESCALE_{U}); }}
#[test] fn tick_{ty}() {{ run_ticks::<{ty}>("{ty}", TICK_{U}); }}''')
open("tests/common_cases.rs", "w").write("\n".join(out) + "\n")
print("rows:", {k: len(v) for k, v in [("lines", out)]})
