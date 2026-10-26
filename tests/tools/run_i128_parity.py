#!/usr/bin/env python3
"""Freeze and compare current/baseline i128 arithmetic with D96 at 8/10/12dp."""
import argparse
import collections
import datetime
from decimal import Decimal, ROUND_DOWN, localcontext
import hashlib
import html
import json
import math
import os
from pathlib import Path
import platform
import random
import re
import shutil
import statistics
import subprocess
import tarfile
import tempfile
import time

import run_decimal_comparison as common

if not __debug__:
    raise RuntimeError("run without -O or PYTHONOPTIMIZE so benchmark checks remain enabled")

ROOT, CRATE = common.ROOT, common.CRATE
BASELINE = "3f6e82a568dee9f3dde619bc6ac02b7f4448ab58"
FIXDEC = "a853c0c42c003588826f69586aa29e36aa72f8c9"
SEED = 0x4931323850415249
WORKLOADS = [f"{family}_{sign}" for family in ("small", "amounts", "rate")
             for sign in ("positive", "mixed_sign")]


def write(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def require_default_profile(environment):
    flags = {"RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_RUSTFLAGS"}
    overridden = [key for key, value in environment.items() if value.strip() and (
        key in flags or key.startswith("CARGO_PROFILE_RELEASE_") or
        key.startswith("CARGO_TARGET_") and key.endswith("_RUSTFLAGS"))]
    if overridden:
        raise ValueError("controlled build requires manifest profile defaults; unset " + ", ".join(sorted(overridden)))


def fixtures():
    rng = random.Random(SEED)
    bounds = {"small": (500_000, 1_500_000, 500_000, 1_500_000),
              "amounts": (1_000_000, 100_000_000_000_000, 10_000, 10_000_000),
              "rate": (1_000_000, 100_000_000_000_000, 10, 10_000)}
    cases = []
    for family, (alo, ahi, blo, bhi) in bounds.items():
        pairs = [(alo, blo), (ahi, blo), (ahi - 1, bhi), (alo + 1, bhi - 1)]
        if family == "small":
            pairs += [(1_000_001, 500_000), (1_000_016, 1_280_000),
                      (1_000_004, 1_280_000), (1_000_002, 1_048_576)]
        pairs += [(rng.randrange(alo, ahi + 1), rng.randrange(blo, bhi + 1))
                  for _ in range(1024 - len(pairs))]
        for sign in ("positive", "mixed_sign"):
            for i, (a, b) in enumerate(pairs):
                if sign == "mixed_sign":
                    a *= -1 if i % 4 < 2 else 1
                    b *= -1 if i % 2 else 1
                for dp in (8, 10, 12):
                    f = 10 ** dp
                    want = [(a + b) * 10 ** (dp - 6), (a - b) * 10 ** (dp - 6),
                            common.trunc(a * b * f, 10 ** 12), common.trunc(a * f, b)]
                    # Independent arbitrary-precision decimal cross-check of every oracle.
                    with localcontext() as ctx:
                        ctx.prec = 100
                        x, y = Decimal(a) / 10 ** 6, Decimal(b) / 10 ** 6
                        q = Decimal(1).scaleb(-dp)
                        check = [int(v.quantize(q, rounding=ROUND_DOWN) * f)
                                 for v in (x + y, x - y, x * y, x / y)]
                        assert check == want
                    assert all(abs(v) * 10 ** (12 - dp) < 2 ** 95 for v in want), "D96 result would overflow"
                    cases.append(dict(dp=dp, workload=f"{family}_{sign}", a=str(a), b=str(b),
                                      trunc=list(map(str, want))))
    return dict(schema=3, seed=SEED, description=(
        "1024 shared exact6dp input pairs per workload at targets8/10/12. "
        "Small magnitudes .5..1.5; amounts lhs1..1e8 with rhs.01..10; rate lhs1..1e8 with rhs.00001...01. "
        "Positive and balanced mixed signs; all checked results fit D96. "
        "Primary arithmetic truncates toward zero, with native result objects retained in the timer. "
        "This fixture does not qualify arbitrary18dp operands or full-range overflow fallbacks."), cases=cases)


def checked(cmd, log):
    """Keep stdout/stderr on disk even if an invocation is interrupted or fails."""
    cmd = list(map(str, cmd))
    start = time.monotonic()
    code, error = None, None
    stdout, stderr = log.with_suffix(".stdout"), log.with_suffix(".stderr")
    try:
        with stdout.open("w") as out, stderr.open("w") as err:
            code = subprocess.run(cmd, stdout=out, stderr=err).returncode
        if code:
            raise RuntimeError(f"command exited {code}; see {log}")
        return stdout.read_text()
    except BaseException as exc:
        error = repr(exc)
        raise
    finally:
        write(log, dict(command=cmd, exit_code=code, error=error,
                        wall_seconds=time.monotonic() - start,
                        stdout=stdout.name, stderr=stderr.name))


def source_paths():
    tracked = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).decode().split("\0")
    paths = {ROOT / p for p in tracked if p and (ROOT / p).is_file()}
    paths |= set(CRATE.rglob("*.rs"))
    paths |= set((ROOT / "tests").rglob("*.rs"))
    paths |= {CRATE / "Cargo.toml", CRATE / "Cargo.lock", Path(__file__).resolve(), Path(common.__file__).resolve()}
    return sorted(p for p in paths if "target" not in p.relative_to(ROOT).parts)


def dependency_sources(metadata):
    result = {}
    for package in metadata["packages"]:
        base = Path(package["manifest_path"]).parent.resolve()
        if base in (ROOT, CRATE):
            paths = list((base / "src").rglob("*")) + [base / "Cargo.toml"]
            paths += [p for p in (base / "README.md", base / "build.rs", base / "Cargo.lock") if p.exists()]
        else:
            paths = [p for p in base.rglob("*") if not {".git", "target"}.intersection(p.relative_to(base).parts)]
        hashes = {str(p.relative_to(base)): common.digest(p) for p in sorted(paths) if p.is_file()}
        result[package["id"]] = dict(source=package["source"], manifest=str(base / "Cargo.toml"), sha256=hashes)
    assert any(BASELINE in (p["source"] or "") for p in result.values()), "wrong or missing baseline source"
    assert any(FIXDEC in (p["source"] or "") for p in result.values()), "wrong or missing D96 source"
    return result


def verify_sources(hashes, dependencies):
    actual = {str(p.relative_to(ROOT)): common.digest(p) for p in source_paths()}
    if actual != hashes:
        changed = sorted(k for k in actual.keys() | hashes.keys() if actual.get(k) != hashes.get(k))
        raise RuntimeError("repository sources changed: " + ", ".join(changed))
    for package in dependencies.values():
        base = Path(package["manifest"]).parent
        for rel, sha in package["sha256"].items():
            if common.digest(base / rel) != sha:
                raise RuntimeError("dependency source changed: " + str(base / rel))


def stability(values):
    median = statistics.median(values)
    mad = statistics.median(abs(v - median) for v in values)
    spread = 100 * (max(values) - min(values)) / median
    return dict(median_ns=median, mad_ns=mad, spread_pct=spread,
                noisy=spread > 20 or mad / median > .05)


def host_flags(state):
    flags = []
    if state["loadavg"][0] > max(1, (os.cpu_count() or 1) * .5):
        flags.append("one_minute_load_above_half_logical_cpus")
    for cmd in state["commands"]:
        if cmd["exit_code"]:
            flags.append("host_probe_failed:" + cmd["command"][0])
        if cmd["command"][0] == "pmset":
            limits = re.findall(r"(?:CPU_Speed_Limit|CPU_Scheduler_Limit)\s*=\s*(\d+)", cmd["stdout"])
            if any(int(v) < 100 for v in limits):
                flags.append("thermal_or_scheduler_limit")
        if cmd["command"][0] == "ps":
            for line in cmd["stdout"].splitlines():
                fields = line.split(None, 2)
                if len(fields) == 3 and fields[0].isdigit():
                    try:
                        busy = float(fields[1]) >= 20
                    except ValueError:
                        continue
                    if int(fields[0]) != os.getpid() and busy:
                        flags.append(f"competing_cpu:{fields[0]}:{fields[2]}")
    return sorted(set(flags))


def ratios(summary):
    by_key = {(r["variant"], r["workload"], r["dp"], r["op"]): r for r in summary}
    result = []
    for row in summary:
        if row["variant"] not in ("current_const", "current_oob"):
            continue
        key = (row["workload"], row["dp"], row["op"])
        baseline = by_key[(row["variant"].replace("current_", "baseline_"), *key)]
        rounded = by_key[("D96", *key)]
        native = by_key[("D96Native", *key)]
        d96 = min((rounded, native), key=lambda r: r["median_ns"]) if row["op"] == "div" else rounded
        item = {k: row[k] for k in ("variant", "workload", "dp", "op")}
        item.update(current_ns=row["median_ns"], baseline_ns=baseline["median_ns"], D96_ns=d96["median_ns"],
                    D96_rounded_ns=rounded["median_ns"], D96_native_ns=native["median_ns"], selected_D96_variant=d96["variant"],
                    current_over_baseline=row["median_ns"] / baseline["median_ns"],
                    current_over_D96=row["median_ns"] / d96["median_ns"],
                    current_over_D96_rounded=row["median_ns"] / rounded["median_ns"],
                    current_over_D96_native=row["median_ns"] / native["median_ns"],
                    D96_rounded_noisy=rounded["noisy"], D96_native_noisy=native["noisy"],
                    noisy=any(r["noisy"] for r in (row, baseline, d96)))
        for name, control in (("baseline", baseline), ("D96", d96), ("D96_rounded", rounded), ("D96_native", native)):
            paired = [a / b for a, b in zip(row["raw_ns"], control["raw_ns"])]
            item[f"paired_current_over_{name}"] = paired
            item[f"paired_median_current_over_{name}"] = statistics.median(paired)
        result.append(item)
    return result


def render(output):
    identity = json.loads((output / "identity.json").read_text())
    summary = common.summarize(output)
    for row in summary:
        row.update(stability(row["raw_ns"]))
    write(output / "summary.json", summary)
    comparisons = ratios(summary)
    write(output / "ratios.json", comparisons)
    hosts = [json.loads(p.read_text()) for p in sorted(output.glob("host-*.json"))]
    flags = sorted({flag for host in hosts for flag in host["flags"]})
    primary = [r for r in comparisons if r["op"] in ("mul", "div")]
    assessment = dict(qualification="diagnostic-unpinned-shared-host", parity_passed=False,
                      target_observed=all(r["current_over_D96"] <= 1.10 for r in primary),
                      primary_max_current_over_D96=max(r["current_over_D96"] for r in primary),
                      primary_max_current_over_D96_rounded=max(r["current_over_D96_rounded"] for r in primary),
                      primary_max_current_over_D96_native=max(r["current_over_D96_native"] for r in primary),
                      D96_control_policy="division selects the lower full-campaign median of D96 checked_div_rounded and D96Native checked_div with ToZero reduction only at8/10dp; native12 mul/div results return directly at12dp; add/sub/mul use original D96 only; paired ratios keep the selected path fixed across rounds",
                      primary_max_current_over_baseline=max(r["current_over_baseline"] for r in primary),
                      all_ops_max_current_over_baseline=max(r["current_over_baseline"] for r in comparisons),
                      noisy_primary_cells=sum(r["noisy"] for r in primary), host_flags=flags,
                      limits=["CPU affinity and frequency were not controlled", "ps CPU percentages and load averages are interference diagnostics, not CPU isolation proof"],
                      spread_rule="noisy if full-range spread>20% or MAD/median>5%; no samples discarded")
    write(output / "assessment.json", assessment)
    e = html.escape
    body = []
    for row in comparisons:
        fields = [row["variant"], row["workload"], row["dp"], row["op"],
                  f'{row["current_ns"]:.3f}', f'{row["baseline_ns"]:.3f}', f'{row["D96_rounded_ns"]:.3f}', f'{row["D96_native_ns"]:.3f}', row["selected_D96_variant"],
                  f'{row["current_over_baseline"]:.3f}', f'{row["current_over_D96_rounded"]:.3f}', f'{row["current_over_D96_native"]:.3f}', f'{row["current_over_D96"]:.3f}', str(row["noisy"])]
        body.append("<tr>" + "".join(f"<td>{e(str(v))}</td>" for v in fields) + "</tr>")
    report = f'''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>i128 and D96 parity</title>
<style>body{{font:15px system-ui;max-width:1450px;margin:32px auto;padding:0 20px;color:#1e293b}}p,li{{max-width:110ch;line-height:1.6}}table{{border-collapse:collapse;width:100%;font-size:13px}}td,th{{padding:8px;border:1px solid #cbd5e1;text-align:right}}td:first-child,th:first-child{{text-align:left}}th{{background:#e2e8f0}}pre{{overflow:auto;background:#f1f5f9;padding:12px}}.note{{background:#fff7ed;border-left:4px solid #ea580c;padding:16px}}</style>
<h1>i128 arithmetic versus its frozen baseline and D96</h1><p class="note">Diagnostic shared-host throughput. Parity is not qualified by this runner. Noisy cells and competing activity remain visible; every raw sample is retained. Target observed across all primary multiplication/division cells: {assessment['target_observed']}. Maximum current/D96 median ratio: {assessment['primary_max_current_over_D96']:.3f}. Noisy primary cells: {assessment['noisy_primary_cells']}.</p>
<p>Six variants × three output scales8/10/12 × six workloads × four operations =432 cells, each repeated {identity['runs']} times for at least {identity['target_ms']}ms. Ratios divide median ns/op, so values below1 favor current. Paired per-round ratios, full ranges, MAD, and raw observations are in JSON.</p>
<ul><li>All libraries receive identical exact6dp inputs. Small operands have magnitude.5..1.5; amounts use lhs1..1e8 and rhs.01..10; rates use lhs1..1e8 and rhs.00001...01. Positive and balanced mixed signs are separate. Results fit every representation.</li>
<li>All output semantics truncate toward zero at the target precision. Checked add/sub need no further reduction because exact6dp inputs already lie on target grids. pfpd const targets and D96 targets are compiler-known; OOB scales are opaque runtime values.</li>
<li>Both D96 paths use checked native12 multiplication, returning the result directly for target12 and applying public ToZero reduction for targets8/10; exact6dp inputs make this equivalent. D96 division uses checked_div_rounded with ToZero directly at the target. D96Native division uses checked_div at native12, returning directly for target12 and applying public ToZero reduction for targets8/10. All native12 results fit this workload and nested truncation commutes. No handwritten replacement decimal arithmetic is timed.</li>
<li>Division parity uses the faster full-campaign median of the two declared D96 public API paths for each cell. Both paths and ratios remain visible. Paired ratios keep that selected path fixed for every round. Add/sub/mul use original D96 only, avoiding selection among duplicate arithmetic samples.</li>
<li>Inputs are preconstructed. Both operands and each native result pass through black_box. Timers include independent arithmetic and result destruction; parsing, extraction, validation, setup, and printing are untimed. Cells rotate and reverse order between rounds.</li>
<li>Primary operations are multiplication/division. Addition/subtraction are regression controls. This matrix does not qualify arbitrary18dp inputs, every rounding mode, overflow boundaries, or wide fallbacks; production correctness gates remain separate.</li></ul>
<h2>Median ns/op and ratios</h2><table><thead><tr><th>Current type</th><th>Workload</th><th>dp</th><th>Op</th><th>Current</th><th>Baseline</th><th>D96 rounded</th><th>D96 native</th><th>Selected D96</th><th>Current/baseline</th><th>Current/rounded</th><th>Current/native</th><th>Current/selected</th><th>Noisy</th></tr></thead><tbody>{''.join(body)}</tbody></table>
<h2>Assessment and host flags</h2><pre>{e(json.dumps(assessment,indent=2))}</pre><h2>Reproduce and inspect</h2><p><code>python3 tests/tools/run_i128_parity.py --output NEW_DIRECTORY --runs {identity['runs']} --target-ms {identity['target_ms']}</code></p><ul>
<li><a href="identity.json">Identity and source hashes</a></li><li><a href="fixtures.json">Exact inputs and integer oracles</a></li><li><a href="validation.jsonl">Per-cell validation</a></li><li><a href="summary.json">All raw samples and stable spreads</a></li><li><a href="ratios.json">Median and paired ratios</a></li><li><a href="dependency-sources.json">Dependency source hashes</a></li><li><a href="reproduction-source.tar.gz">Exact repository and pinned baseline/D96 sources</a></li><li><a href="manifest.json">Evidence hashes</a></li></ul><details><summary>Identity</summary><pre>{e(json.dumps(identity,indent=2))}</pre></details></html>'''
    (output / "report.html").write_text(report)


def execute(args, output):
    fixture_path = output / "fixtures.json"
    fixture_path.write_text(json.dumps(fixtures(), separators=(",", ":")) + "\n")
    if args.fixtures_only:
        return
    metadata = json.loads(checked(["cargo", "metadata", "--manifest-path", CRATE / "Cargo.toml", "--locked", "--format-version", "1"], output / "cargo-metadata-log.json"))
    write(output / "cargo-metadata.json", metadata)
    dependencies = dependency_sources(metadata)
    write(output / "dependency-sources.json", dependencies)
    hashes = {str(p.relative_to(ROOT)): common.digest(p) for p in source_paths()}
    write(output / "source-before.json", hashes)
    build = checked(["cargo", "build", "--release", "--locked", "--manifest-path", CRATE / "Cargo.toml", "--bin", "i128_parity", "--target-dir", ROOT / "target/decimal-comparison", "--message-format=json-render-diagnostics"], output / "build-log.json")
    verify_sources(hashes, dependencies)
    exes = [m["executable"] for line in build.splitlines() if (m := json.loads(line)).get("reason") == "compiler-artifact" and m.get("target", {}).get("name") == "i128_parity" and m.get("executable")]
    assert len(exes) == 1, "Cargo must identify exactly one benchmark executable"
    binary = output / "decimal-comparison.bin"
    shutil.copy2(exes[0], binary)
    binary.chmod(0o555)
    identity = dict(utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), runs=args.runs, target_ms=args.target_ms,
                    profile="manifest defaults: release opt3, noLTO, codegen-units1", qualification="diagnostic-unpinned-shared-host",
                    baseline_rev=BASELINE, fixdec_rev=FIXDEC, platform=platform.platform(), cpu_count=os.cpu_count(),
                    fixture_sha256=common.digest(fixture_path), binary_sha256=common.digest(binary), built_executable=exes[0],
                    runner_sha256=common.digest(Path(__file__).resolve()), source_sha256=hashes,
                    dependency_manifest_sha256=common.digest(output / "dependency-sources.json"),
                    commands=[common.command(c) for c in (["rustc", "-Vv"], ["cargo", "-V"], ["git", "-C", str(ROOT), "rev-parse", "HEAD"], ["git", "-C", str(ROOT), "status", "--short"])],
                    environment={k: os.environ[k] for k in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "RUSTUP_TOOLCHAIN", "CARGO_BUILD_TARGET", "CARGO_BUILD_RUSTFLAGS", "CARGO_PROFILE_RELEASE_LTO", "CARGO_PROFILE_RELEASE_OPT_LEVEL", "CARGO_PROFILE_RELEASE_CODEGEN_UNITS") if k in os.environ},
                    initial_host=common.host_state())
    write(output / "identity.json", identity)
    with tarfile.open(output / "reproduction-source.tar.gz", "w:gz") as archive:
        for rel in hashes:
            archive.add(ROOT / rel, arcname=rel, recursive=False)
        for package in dependencies.values():
            if any(rev in (package["source"] or "") for rev in (BASELINE, FIXDEC)):
                base = Path(package["manifest"]).parent
                label = "baseline" if BASELINE in package["source"] else "fixdec"
                for rel in package["sha256"]:
                    archive.add(base / rel, arcname=f"pinned-sources/{label}/{rel}", recursive=False)
    verify_sources(hashes, dependencies)
    listing = checked([binary, "list", fixture_path], output / "list-log.json")
    cells = json.loads(listing)
    assert len(cells) == 432 and len({r["id"] for r in cells}) == 432
    write(output / "cells.json", cells)
    validation = checked([binary, "validate", fixture_path], output / "validation-log.json")
    (output / "validation.jsonl").write_text(validation)
    ids = {r["id"] for r in cells}
    vrows = [json.loads(line) for line in validation.splitlines()]
    common.verify_validation(vrows, ids)
    assert all(r["mismatches"] == 0 and r["checked_pairs"] == 1024 for r in vrows)
    if args.validate_only:
        verify_sources(hashes, dependencies)
        assert common.digest(binary) == identity["binary_sha256"] and common.digest(fixture_path) == identity["fixture_sha256"]
        return
    evidence = dict(identity_sha256=common.digest(output / "identity.json"), binary_sha256=identity["binary_sha256"], fixture_sha256=identity["fixture_sha256"], source_manifest_sha256=common.digest(output / "source-before.json"), runner_sha256=identity["runner_sha256"])
    print(f"Validated432 cells; {args.runs} rounds, at least{args.target_ms}ms per cell", flush=True)
    for n in range(args.runs):
        verify_sources(hashes, dependencies)
        assert common.digest(binary) == identity["binary_sha256"] and common.digest(fixture_path) == identity["fixture_sha256"]
        before = common.host_state()
        data = checked([binary, "bench", fixture_path, n, args.target_ms], output / f"round-{n:02d}-log.json")
        after = common.host_state()
        rows = [json.loads(line) for line in data.splitlines()]
        assert len(rows) == len(ids) and {r["id"] for r in rows} == ids
        for row in rows:
            assert row["round"] == n and row["iterations"] > 0 and row["iterations"] % 1024 == 0
            assert row["wall_ns"] >= args.target_ms * 1_000_000 and math.isclose(row["ns_per_op"], row["wall_ns"] / row["iterations"], rel_tol=1e-12)
            row["evidence"] = evidence
        (output / f"round-{n:02d}.jsonl").write_text("".join(json.dumps(r, sort_keys=True) + "\n" for r in rows))
        write(output / f"host-{n:02d}.json", dict(before=before, after=after, flags=sorted(set(host_flags(before) + host_flags(after)))))
        print(f"Round{n+1}/{args.runs}:432 samples retained", flush=True)
    verify_sources(hashes, dependencies)
    assert common.digest(binary) == identity["binary_sha256"] and common.digest(fixture_path) == identity["fixture_sha256"]
    render(output)


def self_test():
    require_default_profile({})
    for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_PROFILE_RELEASE_LTO", "CARGO_PROFILE_RELEASE_OPT_LEVEL", "CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS"):
        try:
            require_default_profile({key: "override"})
        except ValueError as exc:
            assert key in str(exc)
        else:
            raise AssertionError("profile override accepted: " + key)
    f = fixtures()
    assert len(f["cases"]) == 18 * 1024
    assert collections.Counter((c["workload"], c["dp"]) for c in f["cases"]) == collections.Counter({(w, dp): 1024 for w in WORKLOADS for dp in (8, 10, 12)})
    assert not stability([10, 10, 10])["noisy"] and stability([10, 10, 100])["noisy"]
    rows = [dict(variant=name, workload="small_positive", dp=8, op=op, median_ns=value, raw_ns=[value]*7, noisy=False)
            for op in ("mul", "div")
            for name, value in (("current_const", 8), ("baseline_const", 10), ("current_oob", 12), ("baseline_oob", 10), ("D96", 10), ("D96Native", 5))]
    for row in rows:
        if row["variant"] == "D96Native" and row["op"] == "div":
            row["raw_ns"] = [5] * 4 + [20] * 3
    rr = ratios(rows)
    assert rr[0]["current_over_D96"] == .8 and rr[1]["current_over_baseline"] == 1.2
    assert all(r["selected_D96_variant"] == ("D96Native" if r["op"] == "div" else "D96") for r in rr)
    assert rr[2]["current_over_D96"] == 1.6 and rr[2]["current_over_D96_rounded"] == .8
    assert rr[2]["paired_current_over_D96"] == rr[2]["paired_current_over_D96_native"] == [1.6] * 4 + [.4] * 3
    slower_native = [{**r, "median_ns": 20, "raw_ns": [20] * 7} if r["variant"] == "D96Native" else r for r in rows]
    assert all(r["selected_D96_variant"] == "D96" for r in ratios(slower_native))
    fake = dict(loadavg=[0,0,0], commands=[dict(command=["ps"], exit_code=0, stdout="999999 90.0 rustc")])
    assert host_flags(fake) == ["competing_cpu:999999:rustc"]
    with tempfile.TemporaryDirectory(prefix="i128-parity-self-test-") as td:
        output = Path(td)
        (output / "fixtures.json").write_text("synthetic self-test fixture")
        (output / "decimal-comparison.bin").write_text("synthetic self-test binary")
        write(output / "identity.json", dict(runs=7, target_ms=10,
            fixture_sha256=common.digest(output / "fixtures.json"),
            binary_sha256=common.digest(output / "decimal-comparison.bin")))
        samples = [dict(id=r["variant"] + "/" + r["op"], variant=r["variant"], workload=r["workload"], dp=r["dp"],
                        op=r["op"], mode="matched", ns_per_op=r["median_ns"]) for r in rows]
        write(output / "cells.json", samples)
        (output / "validation.jsonl").write_text("".join(json.dumps(dict(id=r["id"], mismatches=0)) + "\n" for r in samples))
        for n in range(7):
            (output / f"round-{n:02d}.jsonl").write_text("".join(json.dumps(r) + "\n" for r in samples))
        render(output)
        assessment = json.loads((output / "assessment.json").read_text())
        assert assessment["primary_max_current_over_D96"] == 2.4 and not assessment["target_observed"]
        assert assessment["primary_max_current_over_D96_rounded"] == 1.2
        assert not assessment["parity_passed"] and (output / "report.html").is_file()
    print("Profile rejection, fixture/Decimal, ratio, noise, host-flag, and report self-checks passed")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--output", type=Path)
    p.add_argument("--runs", type=int, default=7)
    p.add_argument("--target-ms", type=float, default=10)
    p.add_argument("--fixtures-only", action="store_true")
    p.add_argument("--validate-only", action="store_true")
    p.add_argument("--report-only", action="store_true")
    p.add_argument("--self-test", action="store_true")
    args = p.parse_args()
    if args.self_test:
        self_test(); return
    if args.runs < 7 or not math.isfinite(args.target_ms) or args.target_ms < 10:
        p.error("at least7 rounds and10ms per cell are required")
    if args.report_only:
        if args.output is None:
            p.error("--report-only requires --output")
        output = args.output.resolve()
        manifest = json.loads((output / "manifest.json").read_text())
        for rel, entry in manifest.items():
            if rel not in ("report.html", "summary.json", "ratios.json", "assessment.json") and common.digest(output / rel) != entry["sha256"]:
                raise RuntimeError("immutable evidence changed: " + rel)
        render(output); common.write_manifest(output); return
    if not args.fixtures_only:
        require_default_profile(os.environ)
    if args.output is None:
        (ROOT / "artifacts").mkdir(exist_ok=True)
        output = Path(tempfile.mkdtemp(prefix="i128-parity-" + time.strftime("%Y%m%d-%H%M%S") + "-", dir=ROOT / "artifacts"))
    else:
        output = args.output.resolve()
        output.mkdir(parents=True, exist_ok=True)
        if any(output.iterdir()):
            raise RuntimeError("use a fresh empty output directory")
    print("Evidence:", output, flush=True)
    try:
        execute(args, output)
        write(output / "status.json", dict(status="fixtures-only" if args.fixtures_only else "validated-only" if args.validate_only else "complete"))
    except BaseException as exc:
        write(output / "status.json", dict(status="failed", error=repr(exc)))
        raise
    finally:
        common.write_manifest(output)


if __name__ == "__main__":
    main()
