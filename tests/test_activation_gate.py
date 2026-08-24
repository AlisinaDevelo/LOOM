from __future__ import annotations

import json
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class ActivationGateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.gate = json.loads((ROOT / "benchmarks/retrieval/v0/gate.json").read_text())
        cls.gate_doc = (ROOT / "docs/ACTIVATION_GATE.md").read_text()
        cls.worksheet = (ROOT / "docs/studies/v0.1-participant-worksheet.md").read_text()

    def test_gate_is_explicit_and_not_claimed_as_measured(self) -> None:
        self.assertEqual("hypothesis", self.gate["status"])
        self.assertEqual("not_run", self.gate["measurement_status"])
        thresholds = self.gate["thresholds"]
        self.assertEqual(0.8, thresholds["exact_source_recall_at_1_min"])
        self.assertEqual(0.95, thresholds["exact_source_recall_at_5_min"])
        self.assertEqual(0.9, thresholds["evidence_open_success_min"])
        self.assertEqual(1000, thresholds["p95_latency_ms_max"])
        self.assertEqual(0.98, thresholds["index_completeness_min"])
        self.assertEqual(1.0, thresholds["no_result_disclosure_min"])

    def test_participant_bounds_and_fixture_are_rights_clean(self) -> None:
        study = self.gate["participant_study"]
        self.assertEqual(12, study["minimum_participants"])
        self.assertEqual(20, study["maximum_participants"])
        self.assertLessEqual(study["minimum_completed_participants"], study["minimum_participants"])
        self.assertTrue(self.gate["fixture_rights_clean"])
        self.assertTrue((ROOT / self.gate["fixture_manifest"]).is_file())
        self.assertTrue((ROOT / study["worksheet"]).is_file())

    def test_decisions_and_claim_traceability_are_present(self) -> None:
        for decision in ("advance", "narrow", "stop"):
            self.assertTrue(self.gate["decisions"][decision])
            self.assertIn(decision, self.gate_doc.lower())
        self.assertIn("README.md", self.gate_doc)
        self.assertIn("docs/EVALUATION.md", self.gate_doc)
        self.assertIn("docs/PRODUCT.md", self.gate_doc)
        self.assertIn("docs/ROADMAP.md", self.gate_doc)

    def test_worksheet_minimizes_private_data_and_tracks_failure_classes(self) -> None:
        for required in (
            "P__",
            "Consent confirmed",
            "Evidence-open successes",
            "Missing-index failures",
            "Wrong-source failures",
            "Evidence-viewer failures",
            "Data deletion confirmed",
        ):
            self.assertIn(required, self.worksheet)
        for forbidden in ("raw source text", "screenshots", "credentials", "private documents"):
            self.assertIn(forbidden, self.worksheet)


if __name__ == "__main__":
    unittest.main()
