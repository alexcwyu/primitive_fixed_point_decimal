#!/usr/bin/env python3
"""Pinned, interleaved min-of-N arithmetic benchmark and performance regressions.
Run from the crate root: python3 tests/tools/bench_arithmetic.py --runs 7
No timing assertions are included in cargo test: use a quiet, fixed-frequency CPU.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--runs', type=int, default=7)
parser.add_argument('--cpu', type=int, default=2)
parser.add_argument('--scratch', type=Path, default=Path(tempfile.gettempdir()) / 'pfpd-bench-arithmetic')
args = parser.parse_args()
if args.runs < 5:
    parser.error('at least five interleaved runs are required')
args.scratch.mkdir(parents=True, exist_ok=True)
target = args.scratch / 'target'
subprocess.run(['cargo', 'build', '--release', '--target-dir', str(target)], check=True)
deps = target / 'release/deps'
rlib = next(deps.glob('libprimitive_fixed_point_decimal-*.rlib'))
source = Path('tests/tools/bench_arithmetic.rs')
binaries = []
for alternative in [True, False]:
    binary = args.scratch / ('alternative' if alternative else 'decimal')
    env = os.environ.copy()
    env.pop('ALT_BUILD', None)
    if alternative:
        env['ALT_BUILD'] = '1'
    subprocess.run(['rustc', '--edition=2021', '-C', 'opt-level=3', '-L', f'dependency={deps}',
                    '--extern', f'primitive_fixed_point_decimal={rlib}', str(source), '-o', str(binary)], env=env, check=True)
    binaries.append(binary)
best = [{}, {}]
pattern = re.compile(r'^(.*?)\s+([0-9.]+) ns/op$')
for _ in range(args.runs):
    for i, binary in enumerate(binaries):
        output = subprocess.check_output(['taskset', '-c', str(args.cpu), str(binary)], text=True)
        for line in output.splitlines():
            match = pattern.match(line)
            if match:
                name, value = match[1].strip(), float(match[2])
                best[i][name] = min(best[i].get(name, float('inf')), value)
print(f'{"benchmark":<42}{"alternative":>12}{"decimal":>10}{"ratio":>8}')
for name, alt in best[0].items():
    actual = best[1][name]
    print(f'{name:<42}{alt:12.2f}{actual:10.2f}{actual / alt:8.2f}')
(args.scratch / 'results.json').write_text(json.dumps(best, indent=2))
# Regression bars for measured problems fixed by the arithmetic reviews.
bars = [('i32 decimal mul', 1.25), ('i32 decimal div positive', 1.25),
        ('i32 decimal div signs', 1.25), ('i32 decimal div wide', 1.25),
        ('i64 rescale grow', 1.25), ('i128 rescale drop', 2.0), ('i128 oob rescale drop', 2.0)]
violations = [name for name, limit in bars if best[1][name] > limit * best[0][name]]
assert not violations, f'decimal API exceeds the correct alternative regression bar: {violations}'
