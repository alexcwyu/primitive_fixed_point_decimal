#!/usr/bin/env python3
"""Reproduce decimal +,-,*,/ comparisons with exact, shared 6/7/8dp fixtures.

Run from any directory. Sources are pinned in decimal_comparison/Cargo.lock.
Generated evidence is append-only under a fresh --output directory.
"""
import argparse
import datetime
import decimal
import hashlib
import html
import json
import os
from pathlib import Path
import platform
import random
import shutil
import statistics
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
CRATE = Path(__file__).with_name("decimal_comparison")
SEED = 0x504650443638


def trunc(n, d):
    assert d != 0
    q = abs(n) // abs(d)
    return -q if (n < 0) != (d < 0) else q


def nearest(n, d):
    q = trunc(n, d)
    if abs(n - q * d) * 2 >= abs(d):
        q += -1 if (n < 0) != (d < 0) else 1
    return q


def make_fixtures():
    cases = []
    rng = random.Random(SEED)
    for dp in (6, 7, 8):
        f = 10 ** dp
        pairs = [(f + 1, f // 2), (2 * f + 1, 2 * f),
                 (f // 5, f // 10), (123 * f + 1, 3 * f + 7), (f, f)]
        pairs += [(rng.randrange(10 * f, 1000 * f),
                   rng.randrange(f // 100, 10 * f)) for _ in range(1024 - len(pairs))]
        for workload in ("positive", "mixed_sign"):
            for i, (a, b) in enumerate(pairs):
                if workload == "mixed_sign":
                    a *= -1 if i % 4 < 2 else 1
                    b *= -1 if i % 2 else 1
                want = [a + b, a - b, trunc(a * b, f), trunc(a * f, b)]
                rounded = [a + b, a - b, nearest(a * b, f), nearest(a * f, b)]
                # Independent decimal implementation checks the integer oracle.
                if i < 16:
                    with decimal.localcontext() as ctx:
                        ctx.prec = 100
                        x, y, quantum = decimal.Decimal(a) / f, decimal.Decimal(b) / f, decimal.Decimal(1) / f
                        vals = [x + y, x - y, x * y, x / y]
                        for mode, expected in [(decimal.ROUND_DOWN, want), (decimal.ROUND_HALF_UP, rounded)]:
                            assert [int(v.quantize(quantum, rounding=mode) * f) for v in vals] == expected
                cases.append(dict(dp=dp, workload=workload, a=str(a), b=str(b),
                                  trunc=list(map(str, want)), round=list(map(str, rounded))))
    assert trunc(-7, 2) == -3 and nearest(-7, 2) == -4
    assert nearest(7, -2) == -4 and nearest(-6, -2) == 3
    return dict(schema=1, seed=SEED, description=(
        "1024 pairs per scale/workload; scales 6,7,8; positive and all four sign combinations. "
        "Random |a| in [10,1000), |b| in [.01,10), plus exact/tie/tiny-digit cases. "
        "Identical exact mantissas for every library; no conversion in timed loops. "
        "Results fit every compared signed representation; positive subtraction is nonnegative. "
        "All quotients have magnitude below 100000."), cases=cases)


def command(cmd, **kwargs):
    p = subprocess.run(cmd, capture_output=True, text=True, **kwargs)
    return dict(command=cmd, exit_code=p.returncode, stdout=p.stdout, stderr=p.stderr)


def host_state():
    commands = [["sysctl", "-n", "vm.loadavg"], ["pmset", "-g", "therm"],
                ["ps", "-A", "-o", "pid=,pcpu=,comm=", "-r"]]
    if platform.system() != "Darwin":
        commands = [["uptime"], ["ps", "-eo", "pid,pcpu,comm", "--sort=-pcpu"]]
    state = dict(utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), loadavg=os.getloadavg())
    state["commands"] = [command(c) for c in commands]
    return state


def checked(cmd, log):
    result = command(list(map(str, cmd)))
    log.write_text(json.dumps(result, indent=2) + "\n")
    if result["exit_code"]:
        raise RuntimeError(f"Command failed; see {log}\n{result['stderr'][-3000:]}")
    return result["stdout"]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_manifest(output):
    manifest = {str(p.relative_to(output)): dict(sha256=digest(p), bytes=p.stat().st_size)
                for p in sorted(output.rglob("*")) if p.is_file() and ".git" not in p.parts and p.name != "manifest.json"}
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def verify_rounds(rounds, ids, count):
    expected_files = {f"round-{n:02d}.jsonl" for n in range(count)}
    if set(rounds) != expected_files:
        raise ValueError("incomplete campaign: wrong round count or filenames")
    for name, rows in rounds.items():
        if len(rows) != len(ids) or {r["id"] for r in rows} != ids:
            raise ValueError(f"incomplete or duplicate cells in {name}")


def verify_validation(rows, ids):
    if len(rows) != len(ids) or {r["id"] for r in rows} != ids:
        raise ValueError("incomplete or duplicate oracle validation")


def summarize(output):
    identity = json.loads((output / "identity.json").read_text())
    ids = {r["id"] for r in json.loads((output / "cells.json").read_text())}
    verify_validation([json.loads(s) for s in (output / "validation.jsonl").read_text().splitlines()], ids)
    if digest(output / "fixtures.json") != identity["fixture_sha256"]:
        raise ValueError("fixture hash differs from recorded campaign identity")
    if digest(output / "decimal-comparison.bin") != identity["binary_sha256"]:
        raise ValueError("binary hash differs from recorded campaign identity")
    rows = []
    rounds = {}
    for p in sorted(output.glob("round-*.jsonl")):
        rounds[p.name] = []
        for line in p.read_text().splitlines():
            row = json.loads(line)
            rounds[p.name].append(row)
            row["round_file"] = p.name
            rows.append(row)
    verify_rounds(rounds, ids, identity["runs"])
    groups = {}
    for row in rows:
        groups.setdefault(row["id"], []).append(row)
    summary = []
    for cell_id, data in sorted(groups.items()):
        vals = [r["ns_per_op"] for r in data]
        median = statistics.median(vals)
        summary.append(dict(**{k: v for k, v in data[0].items()
                              if k not in ("ns_per_op", "iterations", "wall_ns", "elapsed_ns", "round_file")},
                            samples=len(vals), median_ns=median, min_ns=min(vals), max_ns=max(vals),
                            mad_ns=statistics.median(abs(v - median) for v in vals),
                            spread_pct=100 * (max(vals) - min(vals)) / median,
                            raw_ns=vals))
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    return summary


def report(output, summary, identity, validations):
    e = html.escape
    vmap = {x["id"]: x for x in validations}
    rows = []
    for r in summary:
        v = vmap.get(r["id"], {})
        fields = [r.get("variant", r["id"]), r.get("workload", ""), r.get("dp", ""),
                  r.get("mode", ""), r.get("op", ""), r.get("bytes", ""),
                  f'{r["median_ns"]:.3f}', f'{r["min_ns"]:.3f}–{r["max_ns"]:.3f}',
                  f'{r["mad_ns"]:.3f}', r["samples"], v.get("mismatches", "see validation"),
                  v.get("max_error_units", v.get("max_abs_error", ""))]
        rows.append("<tr>" + "".join(f"<td>{e(str(x))}</td>" for x in fields) + "</tr>")
    focused = {}
    for r in summary:
        if r.get("dp") == 8 and r.get("mode") == "matched" and r.get("workload") == "positive" and not r.get("baseline"):
            focused.setdefault(r["variant"], {})[r["op"]] = r["median_ns"]
    overview = '<table><thead><tr><th>Implementation</th><th>+ ns/op</th><th>− ns/op</th><th>× ns/op</th><th>÷ ns/op</th></tr></thead><tbody>'
    for variant, values in sorted(focused.items()):
        overview += '<tr><td>' + e(variant) + '</td>' + ''.join(f'<td>{values[op]:.3f}</td>' for op in ("add", "sub", "mul", "div")) + '</tr>'
    overview += '</tbody></table>'
    target_note = ("Known-target supplement. Competitor output precision and quantization factors are compiler-known constants. "
                   "Const-scale controls remain compiler-known; OOB controls remain runtime-configured. Only matched cells are timed."
                   if identity.get("target_mode") == "known" else
                   "Runtime-target campaign. Const-scale types encode the output scale at compile time. OOB and competitor explicit "
                   "quantization APIs receive an opaque runtime target. Native operators retain their built-in precision. "
                   "Use the separate known-target supplement to assess constant-precision call sites.")
    notes = output / "review.html"
    review = notes.read_text() if notes.exists() else "<p>Review notes are recorded separately in this evidence directory.</p>"
    template = '''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Decimal arithmetic comparison, 6–8 dp</title><style>
:root{font:15px system-ui;color:#1e293b;background:#f8fafc}body{max-width:1500px;margin:32px auto;padding:0 20px}h1,h2{color:#0f172a}p,li{max-width:100ch;line-height:1.6}table{border-collapse:collapse;width:100%;font-size:13px;background:white}th,td{padding:8px 10px;border:1px solid #cbd5e1;text-align:right}th:first-child,td:first-child{text-align:left}thead{position:sticky;top:0;background:#e2e8f0}input,select{padding:8px;margin:4px}.notice{padding:16px;background:#fff7ed;border-left:4px solid #ea580c}code{background:#e2e8f0;padding:2px 4px}pre{overflow:auto;background:#e2e8f0;padding:12px}a{color:#0369a1}summary{cursor:pointer}tr.hidden{display:none}
</style><h1>Decimal arithmetic comparison at 6, 7, and 8 dp</h1>
<p class="notice"><b>Diagnostic Apple Silicon measurements.</b> CPU affinity, frequency locking, and a dedicated quiet host were unavailable. All samples and host snapshots are retained. These results describe this run; they do not qualify a portable speed ranking or production latency.</p>
<p>Measures independent hot-cache arithmetic throughput in ns/op. It does not measure dependent-chain latency, allocation-free end-to-end trading, parsing, formatting, or IPC. Inputs and all representations are prepared before timing. Both operands and the native result pass through <code>black_box</code>; conversion and oracle checks occur outside timing. BigDecimal result allocation and destruction remain inside timing.</p>
<h2>Comparison contract</h2><ul>
<li>Every implementation receives the same deterministic mantissas at each input scale. Positive inputs and mixed signs are reported separately. Unsigned types run positive inputs only.</li>
<li><b>native</b>: ordinary arithmetic and its documented native rounding/output precision. OOB decimal multiplication/division use their default-round checked APIs because decimal-by-decimal operators do not exist.</li>
<li><b>matched</b>: checked arithmetic and output at the requested decimal places, truncating toward zero. Native results with more decimal places are truncated with the library's own API. These are comparable output semantics for decimal implementations; their range and storage still differ.</li>
<li>D64 stores 8dp, D96 stores 12dp at all times. Their 6/7/8dp inputs are exactly padded. Native multiplication/division truncate. Truncation twice is equivalent to truncating once, which avoids inventing a rounded D64 multiplication API.</li>
<li>OOB scales enter through a runtime opaque value. Const scales are compile-time generics. Neither representation stores a per-value scale.</li>
<li><code>fixed::I32F32</code>, <code>fixed::I64F64</code>, and f64 are binary representations. Their deviations from the exact decimal output are diagnostics, not decimal correctness failures.</li>
<li>BigDecimal division uses its default 100 significant digits before output truncation. Its public API has no direct context-bounded division. Matched output semantics do not imply identical internal precision or work.</li>
<li>Each round rotates and reverses cell ordering. Summaries use medians and retain every sample, min, max, and median absolute deviation. No minimum-only winner selection or timing assertion.</li>
</ul><p>Precision is the number of fractional decimal places, not total significant digits. Storage and value ranges are not equivalent across crates. The workload uses ordinary fitting prices/quantities; full-width boundaries are exercised by the correctness review, not the timings.</p>
<h2>Review and correctness</h2>__REVIEW__
<h2>Results</h2><p><label>Filter <input id="search" placeholder="variant, scale, operation…"></label><label>Mode <select id="mode"><option value="">all</option><option>matched</option><option>native</option></select></label><label>Workload <select id="workload"><option value="">all</option><option>positive</option><option>mixed_sign</option></select></label><span id="count"></span></p>
<div style="overflow:auto"><table id="results"><thead><tr><th>Implementation</th><th>Workload</th><th>dp</th><th>Mode</th><th>Op</th><th>Bytes</th><th>Median ns/op</th><th>Min–max</th><th>MAD</th><th>Samples</th><th>Oracle mismatches</th><th>Max error, output units</th></tr></thead><tbody>__ROWS__</tbody></table></div>
<h2>Reproduction and evidence</h2><p>__COMMAND__</p><ul><li><a href="identity.json">Source, compiler, build, host identity</a></li><li><a href="fixtures.json">Exact shared inputs and independent oracle</a></li><li><a href="validation.jsonl">Per-cell numerical validation</a></li><li><a href="summary.json">Every per-cell sample and aggregate</a></li><li><a href="manifest.json">SHA-256 evidence manifest</a></li></ul><details><summary>Identity</summary><pre>__IDENTITY__</pre></details>
<script>const rows=[...document.querySelectorAll('#results tbody tr')];function filter(){const q=document.querySelector('#search').value.toLowerCase(),m=document.querySelector('#mode').value,w=document.querySelector('#workload').value;let n=0;for(const r of rows){const t=r.cells;const show=r.textContent.toLowerCase().includes(q)&&(!m||t[3].textContent===m)&&(!w||t[1].textContent===w);r.classList.toggle('hidden',!show);n+=show;}document.querySelector('#count').textContent=n+' cells';}for(const x of document.querySelectorAll('input,select'))x.addEventListener('input',filter);filter();</script></html>'''
    repro = e("python3 tests/tools/run_decimal_comparison.py --runs " + str(identity["runs"]) +
              " --target-ms " + str(identity["target_ms"]) + " --profile " + identity["profile"] +
              (" --known-target" if identity.get("target_mode") == "known" else ""))
    template = template.replace('<h2>Results</h2>', '<h2>8 dp overview, positive inputs, matched output</h2><p>' + e(target_note) + '</p>' + overview + '<h2>All results</h2>')
    result = (template.replace("__ROWS__", "\n".join(rows)).replace("__REVIEW__", review)
              .replace("__COMMAND__", f"<code>{repro}</code>")
              .replace("__IDENTITY__", e(json.dumps(identity, indent=2))))
    (output / "report.html").write_text(result)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--runs", type=int, default=7)
    parser.add_argument("--target-ms", type=int, default=20)
    parser.add_argument("--profile", choices=("release", "thin"), default="release")
    parser.add_argument("--known-target", action="store_true", help="matched-only supplement with compiler-known competitor output precision")
    parser.add_argument("--fixtures-only", action="store_true")
    parser.add_argument("--report-only", action="store_true")
    args = parser.parse_args()
    if args.runs < 5 or args.target_ms < 5:
        parser.error("at least five balanced rounds and five milliseconds per cell are required")
    output = args.output or ROOT / "artifacts" / ("decimal-comparison-" + time.strftime("%Y%m%d-%H%M%S"))
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    fixtures = output / "fixtures.json"
    if args.report_only:
        identity = json.loads((output / "identity.json").read_text())
        vals = [json.loads(s) for s in (output / "validation.jsonl").read_text().splitlines()]
        manifest_path = output / "manifest.json"
        if manifest_path.exists():
            frozen = json.loads(manifest_path.read_text())
            for name, entry in frozen.items():
                if name in ("identity.json", "fixtures.json", "cells.json", "validation.jsonl",
                            "decimal-comparison.bin", "dependency-sources.json") or name.startswith("round-"):
                    if digest(output / name) != entry["sha256"]:
                        raise ValueError(f"immutable campaign evidence changed: {name}")
        report(output, summarize(output), identity, vals)
        write_manifest(output)
        return
    if fixtures.exists():
        if json.loads(fixtures.read_text()) != make_fixtures():
            raise RuntimeError("existing fixture differs; choose a fresh output directory")
    else:
        fixtures.write_text(json.dumps(make_fixtures(), separators=(",", ":")) + "\n")
    print(f"Fixtures: {fixtures} sha256={digest(fixtures)}", flush=True)
    if args.fixtures_only:
        return
    if (output / "identity.json").exists():
        raise RuntimeError("evidence already exists; choose a fresh output directory")
    metadata = json.loads(checked(["cargo", "metadata", "--manifest-path", CRATE / "Cargo.toml", "--locked",
                                   "--format-version", "1"], output / "cargo-metadata-log.json"))
    (output / "cargo-metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    dependency_sources = {}
    for package in metadata["packages"]:
        base = Path(package["manifest_path"]).parent
        files = [p for p in (base / "src").rglob("*") if p.is_file()]
        files += [base / "Cargo.toml"]
        if (base / "build.rs").exists():
            files.append(base / "build.rs")
        dependency_sources[package["id"]] = dict(source=package["source"], manifest=str(base / "Cargo.toml"),
            sha256={str(p.relative_to(base)): digest(p) for p in sorted(files)})
    (output / "dependency-sources.json").write_text(json.dumps(dependency_sources, indent=2) + "\n")
    source_hashes = {str(p.relative_to(ROOT)): digest(p)
        for p in list(CRATE.rglob("*.rs")) + list((ROOT / "src").rglob("*.rs")) +
        [ROOT / "Cargo.toml", CRATE / "Cargo.toml", CRATE / "Cargo.lock", Path(__file__)]}
    build_stdout = checked(["cargo", "build", "--manifest-path", CRATE / "Cargo.toml", "--locked", "--profile", args.profile,
             "--message-format=json-render-diagnostics", "--target-dir", ROOT / "target/decimal-comparison"], output / "build.json")
    for rel, sha in source_hashes.items():
        if digest(ROOT / rel) != sha:
            raise RuntimeError(f"source changed during build: {rel}")
    for package in dependency_sources.values():
        base = Path(package["manifest"]).parent
        for rel, sha in package["sha256"].items():
            if digest(base / rel) != sha:
                raise RuntimeError(f"dependency source changed during build: {base / rel}")
    built = [m["executable"] for line in build_stdout.splitlines() if (m := json.loads(line)).get("reason") ==
             "compiler-artifact" and m.get("target", {}).get("name") == "decimal-comparison" and m.get("executable")]
    if len(built) != 1:
        raise RuntimeError("Cargo did not identify exactly one benchmark executable")
    built_binary = Path(built[0])
    binary = output / "decimal-comparison.bin"
    shutil.copy2(built_binary, binary)
    identity = dict(utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), runs=args.runs,
                    target_ms=args.target_ms, profile=args.profile, platform=platform.platform(),
                    cpu_count=os.cpu_count(), qualification="diagnostic-unpinned-shared-host",
                    cpu_affinity="not pinned; OS scheduler chooses CPU", fixture_sha256=digest(fixtures),
                    target_mode="known" if args.known_target else "runtime",
                    binary_sha256=digest(binary), source_sha256=source_hashes,
                    built_executable=str(built_binary),
                    dependency_manifest_sha256=digest(output / "dependency-sources.json"),
                    build_environment={k: os.environ[k] for k in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS",
                        "RUSTC", "RUSTC_WRAPPER", "CARGO_BUILD_TARGET", "CARGO_BUILD_RUSTFLAGS",
                        "CARGO_PROFILE_RELEASE_LTO", "CARGO_PROFILE_RELEASE_OPT_LEVEL",
                        "CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "MACOSX_DEPLOYMENT_TARGET",
                        "BIGDECIMAL_DEFAULT_PRECISION", "FASTNUM_DEFAULT_ROUNDING_MODE") if k in os.environ},
                    commands=[command(x) for x in [["rustc", "-Vv"], ["cargo", "-V"],
                        ["git", "-C", str(ROOT), "rev-parse", "HEAD"],
                        ["git", "-C", str(ROOT), "status", "--short"],
                        ["sysctl", "-n", "machdep.cpu.brand_string"]]],
                    initial_host=host_state())
    (output / "identity.json").write_text(json.dumps(identity, indent=2) + "\n")
    suffix = "-known" if args.known_target else ""
    cells = json.loads(checked([binary, "list" + suffix, fixtures], output / "list-log.json"))
    (output / "cells.json").write_text(json.dumps(cells, indent=2) + "\n")
    print(f"Validating {len(cells)} cells before timing", flush=True)
    validation = checked([binary, "validate" + suffix, fixtures], output / "validation-log.json")
    (output / "validation.jsonl").write_text(validation)
    validations = [json.loads(s) for s in validation.splitlines() if s.strip()]
    print(f"Validation passed; beginning {args.runs} rounds", flush=True)
    for n in range(args.runs):
        before = host_state()
        start = time.monotonic()
        data = checked([binary, "bench" + suffix, fixtures, n, args.target_ms], output / f"round-{n:02d}-log.json")
        parsed = [json.loads(s) for s in data.splitlines() if s.strip()]
        if len(parsed) != len(cells) or len({r["id"] for r in parsed}) != len(cells):
            raise RuntimeError("incomplete/duplicate benchmark round")
        if {r["id"] for r in parsed} != {r["id"] for r in cells}:
            raise RuntimeError("cell identity changed during campaign")
        (output / f"round-{n:02d}.jsonl").write_text(data)
        (output / f"host-{n:02d}.json").write_text(json.dumps(dict(before=before, after=host_state()), indent=2) + "\n")
        print(f"Round {n + 1}/{args.runs}: {len(parsed)} cells in {time.monotonic() - start:.1f}s; all retained", flush=True)
    if digest(binary) != identity["binary_sha256"] or digest(fixtures) != identity["fixture_sha256"]:
        raise RuntimeError("binary or fixture changed during campaign")
    for rel, sha in identity["source_sha256"].items():
        if digest(ROOT / rel) != sha:
            raise RuntimeError(f"source changed during campaign: {rel}")
    for package in dependency_sources.values():
        base = Path(package["manifest"]).parent
        for rel, sha in package["sha256"].items():
            if digest(base / rel) != sha:
                raise RuntimeError(f"dependency source changed: {base / rel}")
    report(output, summarize(output), identity, validations)
    write_manifest(output)
    print(f"Report: {output / 'report.html'}", flush=True)


if __name__ == "__main__":
    main()
