"""Shared inputs must preserve each type's own native precision."""
import unittest
import json
from pathlib import Path
import tempfile
from run_requested_types import fixtures, comparisons18, render, common, parity


def summaries18():
    rows = []
    for family in ('const', 'oob'):
        for workload in ('positive', 'mixed_sign'):
            for mode in ('native', 'matched'):
                for op in ('add', 'sub', 'mul', 'div'):
                    for version in ('current', 'baseline'):
                        variant = f'{version}_{family}_i128_scale18'
                        value = (12 if mode == 'native' else 8) if version == 'current' else 10
                        rows.append(dict(id=f'{variant}/{workload}/{mode}/{op}', variant=variant,
                                         workload=workload, mode=mode, op=op, dp=18 if mode == 'native' else 6,
                                         representation_dp=18, exact_decimal=True, raw_ns=[value]*7,
                                         **parity.stability([value]*7)))
    return rows


class RequestedFixtures(unittest.TestCase):
    def test_native_precision_is_distinct_from_matched_precision(self):
        f = fixtures()
        self.assertEqual(len(f['cases']), 2048)
        self.assertEqual(f, fixtures())
        first = f['cases'][0]
        self.assertEqual(first['trunc'][2], '500000')
        self.assertEqual(first['native']['6'][2], '500001')
        self.assertEqual(first['native']['8'][2], '50000050')
        self.assertEqual(first['native']['18'][2], '500000500000000000')

    def test_signs_and_native_scales_are_complete(self):
        rows = fixtures()['cases']
        self.assertEqual(sum(r['workload'] == 'positive' for r in rows), 1024)
        mixed = [r for r in rows if r['workload'] == 'mixed_sign']
        for sa in (-1, 1):
            for sb in (-1, 1):
                self.assertEqual(sum((int(r['a']) > 0) == (sa > 0) and
                                     (int(r['b']) > 0) == (sb > 0) for r in mixed), 256)
        self.assertTrue(all(set(r['native']) == {'6', '8', '10', '12', '18'} for r in rows))

    def test_baseline18_pairs_same_modes_and_retains_every_ratio(self):
        rows = summaries18()
        result = comparisons18(rows + [dict(variant='f64', workload='positive', mode='native', op='add')])
        self.assertEqual(len(result), 32)
        for row in result:
            expected = 1.2 if row['mode'] == 'native' else .8
            self.assertEqual(row['current_over_baseline'], expected)
            self.assertEqual(row['paired_current_over_baseline'], [expected]*7)
            self.assertEqual(row['dp'], 18 if row['mode'] == 'native' else 6)
        with self.assertRaisesRegex(ValueError, 'missing same-mode baseline18'):
            comparisons18([r for r in rows if r is not rows[1]])

    def test_noise_is_reported_without_discarding_samples(self):
        rows = summaries18()
        rows[0]['raw_ns'][-1] = 30
        rows[0].update(parity.stability(rows[0]['raw_ns']))
        row = comparisons18(rows)[0]
        self.assertTrue(row['noisy'])
        self.assertEqual(len(row['paired_current_over_baseline']), 7)
        self.assertEqual(row['paired_current_over_baseline'][-1], 3)

    def test_report_keeps_broad_competitors_and_host_flags(self):
        rows = summaries18()
        for variant, dp, exact in [('f64', None, False), ('fixdec_D64_native8', 8, True), ('fixdec_D96_native12', 12, True)]:
            for workload in ('positive', 'mixed_sign'):
                for mode in ('native', 'matched'):
                    for op in ('add', 'sub', 'mul', 'div'):
                        rows.append(dict(id=f'{variant}/{workload}/{mode}/{op}', variant=variant,
                            workload=workload, mode=mode, op=op, representation_dp=dp,
                            dp=dp if mode == 'native' else 6, exact_decimal=exact, raw_ns=[10]*7,
                            **parity.stability([10]*7)))
        with tempfile.TemporaryDirectory(prefix='requested-types-report-test-') as td:
            output = Path(td)
            (output/'fixtures.json').write_text('synthetic self-test fixture')
            (output/'decimal-comparison.bin').write_text('synthetic self-test binary')
            (output/'identity.json').write_text(json.dumps(dict(runs=7, target_ms=10,
                fixture_sha256=common.digest(output/'fixtures.json'), binary_sha256=common.digest(output/'decimal-comparison.bin'))))
            samples = [{**{k:v for k,v in r.items() if k not in ('raw_ns','median_ns','mad_ns','spread_pct','noisy')},
                        'ns_per_op':r['median_ns']} for r in rows]
            (output/'cells.json').write_text(json.dumps(samples))
            (output/'validation.jsonl').write_text(''.join(json.dumps(dict(id=r['id'], exact_decimal=r['exact_decimal'], mismatches=0, checked_pairs=1024))+'\n' for r in rows))
            for n in range(7):
                (output/f'round-{n:02d}.jsonl').write_text(''.join(json.dumps(r)+'\n' for r in samples))
            (output/'host-00.json').write_text(json.dumps(dict(flags=['synthetic-competing-cpu'])))
            render(output)
            report = (output/'report.html').read_text()
            for variant in ('f64', 'fixdec_D64_native8', 'fixdec_D96_native12'):
                self.assertIn(variant, report)
            assessment = json.loads((output/'assessment18.json').read_text())
            self.assertEqual(assessment['native_comparisons'], 16)
            self.assertEqual(assessment['matched_comparisons'], 16)
            self.assertEqual(assessment['host_flags'], ['synthetic-competing-cpu'])


if __name__ == '__main__': unittest.main()
