#!/usr/bin/env python3
"""Benchmark the requested concrete pfpd types against D64, D96, and f64."""
import argparse
import datetime
from decimal import Decimal, ROUND_DOWN, ROUND_HALF_UP, localcontext
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
import tarfile
import time

import run_decimal_comparison as common
import run_i128_parity as parity

ROOT = common.ROOT
CRATE = common.CRATE
SEED = 0x52514553544544


def fixtures():
    pairs = [(1_000_001, 500_000), (1_000_001, 505_000),
             (1_000_016, 1_280_000), (1_000_004, 1_280_000),
             (1_000_064, 1_048_576), (1_000_002, 1_048_576),
             (1_000_000, 1_000_000), (500_001, 1_499_999)]
    rng = random.Random(SEED)
    pairs += [(rng.randrange(500_000, 1_500_001), rng.randrange(500_000, 1_500_001))
              for _ in range(1024 - len(pairs))]
    cases = []
    for workload in ("positive", "mixed_sign"):
        for i, (a, b) in enumerate(pairs):
            if workload == "mixed_sign":
                a *= -1 if i % 4 < 2 else 1
                b *= -1 if i % 2 else 1

            def values(scale, rounding):
                f = 10 ** scale
                return list(map(str, [(a + b) * 10 ** (scale - 6), (a - b) * 10 ** (scale - 6),
                                     rounding(a * b * f, 10 ** 12), rounding(a * f, b)]))

            row = dict(workload=workload, a=str(a), b=str(b), trunc=values(6, common.trunc),
                       native={str(s): values(s, common.nearest) for s in (6, 8, 10, 12, 18)},
                       native_trunc={str(s): values(s, common.trunc) for s in (8, 12)})
            # Independently check every oracle, including negative division and full native scales.
            with localcontext() as ctx:
                ctx.prec = 100
                x, y = Decimal(a) / 10 ** 6, Decimal(b) / 10 ** 6
                for s, mode, expected in [(6, ROUND_DOWN, row["trunc"])] + [
                    (s, ROUND_HALF_UP, row["native"][str(s)]) for s in (6, 8, 10, 12, 18)] + [
                    (s, ROUND_DOWN, row["native_trunc"][str(s)]) for s in (8, 12)]:
                    quantum = Decimal(1).scaleb(-s)
                    actual = [str(int(v.quantize(quantum, rounding=mode) * 10 ** s))
                              for v in (x + y, x - y, x * y, x / y)]
                    assert actual == expected
            cases.append(row)
    return dict(schema=2, seed=SEED, description=(
        "Same exact six-decimal operands in [.5,1.5], with signed and positive workloads, 1024 pairs each. "
        "Native full-precision oracles distinguish 6/8/10/12/18dp. Matched output truncates to6dp. "
        "Inputs fit every requested representation and all four operation results fit. "
        "This is a homogeneous arithmetic microbenchmark, not an exchange price-times-quantity pipeline."), cases=cases)


def comparisons18(rows):
    by_key = {(r['variant'], r['workload'], r['mode'], r['op']): r for r in rows}
    result = []
    for current in rows:
        if current['variant'] not in ('current_const_i128_scale18', 'current_oob_i128_scale18'):
            continue
        baseline_name = current['variant'].replace('current_', 'baseline_', 1)
        key = (baseline_name, current['workload'], current['mode'], current['op'])
        if key not in by_key:
            raise ValueError('missing same-mode baseline18 control: ' + str(key))
        baseline = by_key[key]
        if not current['raw_ns'] or len(current['raw_ns']) != len(baseline['raw_ns']):
            raise ValueError('unequal current/baseline sample counts')
        paired = [a / b for a, b in zip(current['raw_ns'], baseline['raw_ns'])]
        result.append(dict(variant=current['variant'], workload=current['workload'], mode=current['mode'],
                           dp=current['dp'], representation_dp=18, op=current['op'],
                           current_id=current['id'], baseline_id=baseline['id'],
                           current_ns=current['median_ns'], baseline_ns=baseline['median_ns'],
                           current_over_baseline=current['median_ns'] / baseline['median_ns'],
                           paired_current_over_baseline=paired, paired_median_ratio=statistics.median(paired),
                           current_spread_pct=current['spread_pct'], baseline_spread_pct=baseline['spread_pct'],
                           noisy=current['noisy'] or baseline['noisy']))
    return result


def render(output):
    identity = json.loads((output / "identity.json").read_text())
    rows = common.summarize(output)
    for row in rows:
        row.update(parity.stability(row['raw_ns']))
    (output / 'summary.json').write_text(json.dumps(rows, indent=2) + '\n')
    regression = comparisons18(rows)
    (output / 'regression18.json').write_text(json.dumps(regression, indent=2) + '\n')
    host_flags = sorted({flag for p in output.glob('host-*.json')
                         for flag in json.loads(p.read_text()).get('flags', [])})
    assessment = dict(qualification='diagnostic-unpinned-shared-host',
                      max_current_over_baseline=max(r['current_over_baseline'] for r in regression),
                      noisy_comparisons=sum(r['noisy'] for r in regression),
                      native_comparisons=sum(r['mode'] == 'native' for r in regression),
                      matched_comparisons=sum(r['mode'] == 'matched' for r in regression), host_flags=host_flags)
    (output / 'assessment18.json').write_text(json.dumps(assessment, indent=2) + '\n')
    validation = {r["id"]: r for r in map(json.loads, (output / "validation.jsonl").read_text().splitlines())}
    e = html.escape
    regression_body = []
    for row in regression:
        fields = [row['variant'], row['workload'], row['mode'], row['dp'], row['op'],
                  f"{row['current_ns']:.3f}", f"{row['baseline_ns']:.3f}", f"{row['current_over_baseline']:.3f}",
                  f"{row['current_spread_pct']:.1f}%", f"{row['baseline_spread_pct']:.1f}%", row['noisy']]
        regression_body.append('<tr>' + ''.join(f'<td>{e(str(v))}</td>' for v in fields) + '</tr>')
    tables = []
    for workload in ("positive", "mixed_sign"):
        for mode in ("native", "matched"):
            selected = [r for r in rows if r["workload"] == workload and r["mode"] == mode]
            variants = sorted({r["variant"] for r in selected})
            body = []
            for variant in variants:
                cells = {r["op"]: r for r in selected if r["variant"] == variant}
                numbers = []
                for op in ("add", "sub", "mul", "div"):
                    r = cells[op]
                    numbers.append(f'<td title="range {r["min_ns"]:.3f}–{r["max_ns"]:.3f}; MAD {r["mad_ns"]:.3f}">{r["median_ns"]:.3f}</td>')
                mismatches = sum(validation[r["id"]]["mismatches"] for r in cells.values())
                first = next(iter(cells.values()))
                body.append(f'<tr><td>{e(variant)}</td><td>{first.get("representation_dp", "—")}</td>' +
                            ''.join(numbers) + f'<td>{mismatches}</td></tr>')
            tables.append(f'<h2>{e(workload)} · {e(mode)}</h2><table><thead><tr><th>Type</th><th>Native dp</th><th>+ ns/op</th><th>− ns/op</th><th>× ns/op</th><th>÷ ns/op</th><th>Oracle mismatches</th></tr></thead><tbody>' + ''.join(body) + '</tbody></table>')
    exact_errors = sum(r["mismatches"] for r in validation.values() if r["exact_decimal"])
    total_checks = sum(r["checked_pairs"] for r in validation.values())
    source = f'''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Requested pfpd arithmetic types</title><style>
body{{font:15px system-ui;max-width:1300px;margin:32px auto;padding:0 20px;color:#1e293b;background:#f8fafc}}p,li{{max-width:105ch;line-height:1.6}}table{{border-collapse:collapse;background:white;width:100%;font-size:14px}}th,td{{border:1px solid #cbd5e1;padding:9px;text-align:right}}th:first-child,td:first-child{{text-align:left}}th{{background:#e2e8f0}}.note{{background:#fff7ed;padding:16px;border-left:4px solid #ea580c}}pre{{overflow:auto;background:#e2e8f0;padding:12px}}a{{color:#0369a1}}
</style><h1>Requested concrete decimal types: + − × ÷</h1><p class="note">Diagnostic local Apple Silicon throughput measurements. Shared host, no CPU affinity or locked frequency. Small differences are not qualified speed rankings. These are independent operations over hot cached input pairs, not dependent-chain latency or exchange pipeline timings.</p>
<p>{len(rows)} cells × {identity["runs"]} repetitions = {sum(r["samples"] for r in rows)} retained measurements. {total_checks:,} validation operations; {exact_errors} exact-decimal mismatches. Parsing, construction, conversion, validation, and printing are outside the timer. Both operands and the native result pass through black_box.</p>
<ul><li>All types receive the same exact 6dp decimal inputs, magnitude0.5–1.5. This common range fits ConstScaleFpdec&lt;i32,6&gt;. Positive inputs and balanced mixed signs are separate. Targeted ties exercise native6/8/12/18dp rounding, with separate10dp native oracles.</li>
<li><b>Native:</b> preserve each type's native result precision. pfpd rounds nearest with ties away; D64/D96 truncate at8/12dp. OOB multiplication/division use default-round checked APIs because no decimal-by-decimal operators exist. These rows perform different output semantics.</li>
<li><b>Matched:</b> checked decimal arithmetic, then truncate to6dp inside the timed loop using each library's API. Native result storage stays unchanged. OOB scales are genuinely runtime; const scales and D64/D96 matched target6 are compiler-known.</li>
<li>OOB i64 is measured at8dp. OOB i128 is measured at both12dp, reflecting the updated USD discussion, and18dp. Const i32/6, i64/8, and i128 at8/10/12/18dp are exactly the requested types.</li>
<li>f64 is binary and approximate. Its mismatch counts are diagnostics, not failed decimal tests. Native f64 is inspected at6dp outside timing; matched f64 includes truncation to6dp.</li>
<li>18dp types receive padded6dp operands. Their results retain their full native precision; the workload does not represent arbitrary18dp input data, full integer ranges, or price-times-quantity with different units.</li>
<li>Frozen baseline ConstScaleFpdec&lt;i128,18&gt; and runtime OOB i128/18 use commit {parity.BASELINE}. Their native nearest-away18dp and matched truncate6dp outputs have the same semantics as their current controls. Both versions execute the same benchmark macros in one binary.</li>
<li>Median ns/op is shown; hover over each result for min/max and MAD. Every raw sample, iteration count, ordering, host snapshot, and source/binary hash is retained.</li></ul>
<h2>18dp current versus frozen baseline</h2><p>Ratios are current median ns/op divided by baseline median ns/op; values below1 favor current. Native outputs remain18dp; matched outputs truncate to6dp while retaining18dp storage. Maximum observed ratio: {assessment['max_current_over_baseline']:.3f}. Noisy comparisons: {assessment['noisy_comparisons']}. A cell is flagged when full-range spread exceeds20% or MAD/median exceeds5%; every observation is retained.</p>
<table><thead><tr><th>Current type</th><th>Workload</th><th>Mode</th><th>Output dp</th><th>Op</th><th>Current ns/op</th><th>Baseline ns/op</th><th>Ratio</th><th>Current spread</th><th>Baseline spread</th><th>Noisy</th></tr></thead><tbody>{''.join(regression_body)}</tbody></table><details><summary>Host flags and diagnostic assessment</summary><pre>{e(json.dumps(assessment, indent=2))}</pre></details>
{''.join(tables)}<h2>Reproduce and inspect</h2><p><code>python3 tests/tools/run_requested_types.py --runs {identity["runs"]} --target-ms {identity["target_ms"]}</code></p><ul>
<li><a href="regression18.json">Every18dp median and paired comparison</a></li><li><a href="assessment18.json">18dp spread and host flags</a></li>
<li><a href="identity.json">Compiler, source and binary identity</a></li><li><a href="fixtures.json">Shared exact inputs and full-precision oracles</a></li><li><a href="summary.json">Medians, ranges and every sample</a></li><li><a href="validation.jsonl">Per-cell numerical validation</a></li><li><a href="manifest.json">SHA-256 evidence manifest</a></li><li><a href="reproduction-source.tar.gz">Exact reproduction sources</a></li></ul>
<details><summary>Identity</summary><pre>{e(json.dumps(identity, indent=2))}</pre></details></html>'''
    (output / "report.html").write_text(source)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / 'artifacts' / ('decimal-requested-types-' + time.strftime('%Y%m%d-%H%M%S')))
    parser.add_argument('--runs', type=int, default=7)
    parser.add_argument('--target-ms', type=int, default=20)
    parser.add_argument('--report-only', action='store_true')
    args = parser.parse_args()
    if args.runs < 5 or args.target_ms < 5:
        parser.error('need at least5 repetitions and5ms per cell')
    output = args.output.resolve();output.mkdir(parents=True, exist_ok=True)
    if args.report_only:
        frozen = json.loads((output / 'manifest.json').read_text())
        for rel, entry in frozen.items():
            if rel != 'report.html' and common.digest(output / rel) != entry['sha256']:
                raise ValueError('evidence changed: ' + rel)
        render(output);common.write_manifest(output);return
    if (output / 'identity.json').exists():
        raise ValueError('use a fresh output directory')
    parity.require_default_profile(os.environ)
    fixture = fixtures(); fixture_path = output / 'fixtures.json'
    if fixture_path.exists():
        assert json.loads(fixture_path.read_text()) == fixture
    else:
        fixture_path.write_text(json.dumps(fixture, separators=(',', ':')) + '\n')
    metadata = json.loads(common.checked(['cargo','metadata','--manifest-path',CRATE/'Cargo.toml','--locked','--format-version','1'],output/'metadata-log.json'))
    dependencies = {}
    for p in metadata['packages']:
        base = Path(p['manifest_path']).parent
        paths = [q for q in (base/'src').rglob('*') if q.is_file()] + [base/'Cargo.toml']
        if (base/'build.rs').exists():paths.append(base/'build.rs')
        dependencies[p['id']] = dict(source=p['source'], manifest=p['manifest_path'], sha256={str(q.relative_to(base)):common.digest(q) for q in sorted(paths)})
    (output/'dependency-sources.json').write_text(json.dumps(dependencies,indent=2)+'\n')
    paths = parity.source_paths() + [Path(__file__).resolve(), Path(__file__).with_name('test_requested_types.py').resolve()]
    source_hashes = {str(p.relative_to(ROOT)):common.digest(p) for p in paths}
    def verify_sources():
        for rel, sha in source_hashes.items():assert common.digest(ROOT/rel)==sha,rel
        for p in dependencies.values():
            base=Path(p['manifest']).parent
            for rel,sha in p['sha256'].items():assert common.digest(base/rel)==sha,str(base/rel)
    build=common.checked(['cargo','build','--release','--locked','--manifest-path',CRATE/'Cargo.toml','--bin','requested_types','--target-dir',ROOT/'target/decimal-comparison','--message-format=json-render-diagnostics'],output/'build.json')
    verify_sources()
    exes=[m['executable'] for line in build.splitlines() if (m:=json.loads(line)).get('reason')=='compiler-artifact' and m.get('target',{}).get('name')=='requested_types' and m.get('executable')]
    assert len(exes)==1
    binary=output/'decimal-comparison.bin';shutil.copy2(exes[0],binary)
    identity=dict(utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),runs=args.runs,target_ms=args.target_ms,profile='manifest defaults: release opt3, noLTO, codegen-units1',baseline_rev=parity.BASELINE,helper_sha256=common.digest(Path(parity.__file__).resolve()),platform=platform.platform(),cpu_count=os.cpu_count(),qualification='diagnostic-unpinned-shared-host',fixture_sha256=common.digest(fixture_path),binary_sha256=common.digest(binary),source_sha256=source_hashes,dependency_manifest_sha256=common.digest(output/'dependency-sources.json'),commands=[common.command(c) for c in [['rustc','-Vv'],['cargo','-V'],['git','-C',str(ROOT),'rev-parse','HEAD'],['git','-C',str(ROOT),'status','--short'],['sysctl','-n','machdep.cpu.brand_string']]],initial_host=common.host_state(),environment={k:os.environ[k] for k in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTC_WRAPPER','CARGO_BUILD_TARGET','CARGO_PROFILE_RELEASE_LTO','CARGO_PROFILE_RELEASE_OPT_LEVEL','CARGO_PROFILE_RELEASE_CODEGEN_UNITS'] if k in os.environ})
    (output/'identity.json').write_text(json.dumps(identity,indent=2)+'\n')
    cells=json.loads(common.checked([binary,'list',fixture_path],output/'list-log.json'))
    assert len(cells) == 224 and len({r['id'] for r in cells}) == 224
    (output/'cells.json').write_text(json.dumps(cells,indent=2)+'\n')
    validation=common.checked([binary,'validate',fixture_path],output/'validation-log.json')
    (output/'validation.jsonl').write_text(validation)
    common.verify_validation(list(map(json.loads,validation.splitlines())),{r['id'] for r in cells})
    print(f'Validated {len(cells)} cells; beginning {args.runs} rounds',flush=True)
    for n in range(args.runs):
        before=common.host_state();start=time.monotonic()
        data=common.checked([binary,'bench',fixture_path,n,args.target_ms],output/f'round-{n:02d}-log.json')
        rows=list(map(json.loads,data.splitlines()))
        assert len(rows)==len(cells) and {r['id'] for r in rows}=={r['id'] for r in cells}
        (output/f'round-{n:02d}.jsonl').write_text(data)
        after=common.host_state()
        flags=sorted(set(parity.host_flags(before)+parity.host_flags(after)))
        (output/f'host-{n:02d}.json').write_text(json.dumps(dict(before=before,after=after,flags=flags),indent=2)+'\n')
        print(f'Round {n+1}/{args.runs}: {len(rows)} cells in {time.monotonic()-start:.1f}s; all retained',flush=True)
    verify_sources();assert common.digest(binary)==identity['binary_sha256'];assert common.digest(fixture_path)==identity['fixture_sha256']
    archive_paths=set(subprocess.check_output(['git','ls-files'],cwd=ROOT,text=True).splitlines())|set(source_hashes)
    with tarfile.open(output/'reproduction-source.tar.gz','w:gz') as tar:
        for rel in sorted(archive_paths):tar.add(ROOT/rel,arcname=rel,recursive=False)
    render(output);common.write_manifest(output)
    print('Report:',output/'report.html',flush=True)


if __name__=='__main__':main()
