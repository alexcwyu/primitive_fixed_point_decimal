"""Evidence completeness checks; no timing thresholds."""
import unittest

from run_decimal_comparison import make_fixtures, verify_rounds, verify_validation


class CampaignChecks(unittest.TestCase):
    def test_fixture_oracle_and_determinism(self):
        a = make_fixtures()
        self.assertEqual(a, make_fixtures())
        self.assertEqual(len(a["cases"]), 6144)
        for dp in (6, 7, 8):
            for workload in ("positive", "mixed_sign"):
                self.assertEqual(sum(r["dp"] == dp and r["workload"] == workload for r in a["cases"]), 1024)

    def test_partial_or_duplicate_campaign_cannot_be_reported_as_complete(self):
        full = {f"round-{n:02d}.jsonl": [{"id": "a"}, {"id": "b"}] for n in range(5)}
        verify_rounds(full, {"a", "b"}, 5)
        with self.assertRaises(ValueError):
            verify_rounds(dict(list(full.items())[:-1]), {"a", "b"}, 5)
        full["round-00.jsonl"] = [{"id": "a"}, {"id": "a"}]
        with self.assertRaises(ValueError):
            verify_rounds(full, {"a", "b"}, 5)

    def test_missing_or_duplicate_validation_is_rejected(self):
        verify_validation([{"id": "a"}, {"id": "b"}], {"a", "b"})
        for rows in ([{"id": "a"}], [{"id": "a"}, {"id": "a"}]):
            with self.assertRaises(ValueError):
                verify_validation(rows, {"a", "b"})


if __name__ == "__main__":
    unittest.main()
