from __future__ import annotations

import json
import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class ActivationGateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.gate = json.loads(
            (ROOT / "benchmarks/retrieval/v0/gate.json").read_text(encoding="utf-8")
        )
        cls.gate_doc = (ROOT / "docs/ACTIVATION_GATE.md").read_text(encoding="utf-8")
        cls.worksheet = (ROOT / "docs/studies/v0.1-participant-worksheet.md").read_text(
            encoding="utf-8"
        )

    def test_gate_is_explicit_and_not_claimed_as_measured(self) -> None:
        self.assertEqual("hypothesis", self.gate["status"])
        self.assertEqual("not_run", self.gate["measurement_status"])
        thresholds = self.gate["thresholds"]
        self.assertEqual(0.7, thresholds["activation_rate_min"])
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
        self.assertEqual(12, study["minimum_eligible_participants"])
        self.assertLessEqual(study["minimum_completed_participants"], study["minimum_participants"])
        self.assertTrue(self.gate["fixture_rights_clean"])
        self.assertTrue((ROOT / self.gate["fixture_manifest"]).is_file())
        self.assertTrue((ROOT / study["worksheet"]).is_file())
        manifest_path = ROOT / self.gate["fixture_manifest"]
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        self.assertEqual("CC0-1.0", manifest["license"])
        self.assertIn("synthetic", manifest["notes"].lower())
        self.assertGreater(len(manifest["fixtures"]), 0)
        for fixture in manifest["fixtures"]:
            path = (manifest_path.parent / fixture["path"]).resolve()
            self.assertTrue(path.is_relative_to(manifest_path.parent / "corpus"))
            self.assertTrue(path.is_file())
            self.assertRegex(fixture["content_hash"], r"^blake3:[0-9a-f]{64}$")

    def test_decisions_and_claim_traceability_are_present(self) -> None:
        for decision in ("advance", "narrow", "stop"):
            self.assertTrue(self.gate["decisions"][decision])
            self.assertIn(decision, self.gate_doc.lower())
        self.assertIn("README.md", self.gate_doc)
        self.assertIn("docs/EVALUATION.md", self.gate_doc)
        self.assertIn("docs/PRODUCT.md", self.gate_doc)
        self.assertIn("docs/ROADMAP.md", self.gate_doc)
        linked = set()
        for href in re.findall(r"\[[^\]]+\]\(([^)#]+)(?:#[^)]*)?\)", self.gate_doc):
            if "://" not in href:
                path = (ROOT / "docs" / href).resolve()
                self.assertTrue(path.is_file(), href)
                linked.add(path)
        for document in ("README.md", "docs/EVALUATION.md", "docs/PRODUCT.md", "docs/ROADMAP.md"):
            self.assertIn(ROOT / document, linked)
        for document in ("README.md", "docs/ROADMAP.md"):
            path = ROOT / document
            hrefs = re.findall(r"\[[^\]]+\]\(([^)#]+)(?:#[^)]*)?\)", path.read_text(encoding="utf-8"))
            self.assertIn(ROOT / "docs/ACTIVATION_GATE.md", {
                (path.parent / href).resolve() for href in hrefs if "://" not in href
            })

    def test_rates_define_denominators_and_refuse_empty_measurements(self) -> None:
        metrics = self.gate["metrics"]
        for name in ("activation_rate", "no_result_disclosure"):
            self.assertTrue(metrics[name]["numerator"])
            self.assertTrue(metrics[name]["denominator"])
            self.assertEqual("not_measured", metrics[name]["zero_denominator"])
        self.assertIn("first session", metrics["activation_rate"]["numerator"])
        self.assertIn("including setup and retrieval failures", metrics["activation_rate"]["denominator"])
        self.assertIn("including disclosure failures", metrics["no_result_disclosure"]["denominator"])
        self.assertIn("nonzero denominators", self.gate["decisions"]["advance"])
        for required in ("First-session activation", "0.70", "all completed negative queries", "not measured"):
            self.assertIn(required, self.gate_doc)

    def test_withdrawal_eligibility_and_decision_partition_are_explicit(self) -> None:
        self.assertEqual(["stop", "advance", "narrow"], self.gate["decision_order"])
        denominator = self.gate["metrics"]["activation_rate"]["denominator"]
        for required in ("explicit consent withdrawal", "study-data deletion request", "recorded report cutoff", "both numerator and denominator"):
            self.assertIn(required, denominator)
            self.assertIn(required, re.sub(r"\s+", " ", self.gate_doc))
            self.assertIn(required, re.sub(r"\s+", " ", self.worksheet))
        self.assertIn("superseding decision", self.gate["withdrawal_policy"])
        self.assertIn("12–20 participants enrolled", self.gate["decisions"]["advance"])
        self.assertIn("at least 12 eligible", self.gate["decisions"]["advance"])
        for required in ("No stop condition", "any numeric threshold", "not measured", "cohort eligibility"):
            self.assertIn(required, self.gate["decisions"]["narrow"])

    def test_worksheet_minimizes_private_data_and_tracks_failure_classes(self) -> None:
        for required in (
            "P__",
            "Consent confirmed",
            "Evidence-open successes",
            "Missing-index failures",
            "Wrong-source failures",
            "Evidence-viewer failures",
            "Data deletion confirmed",
            "Setup attempted",
            "Activated",
            "Data withdrawn",
            "Negative queries issued",
            "Negative queries completed",
            "Correct no-result disclosures",
            "Unsupported-result failures",
            "Disclosure failures",
            "not measured",
        ):
            self.assertIn(required, self.worksheet)
        for forbidden in ("raw source text", "screenshots", "credentials", "private documents"):
            self.assertIn(forbidden, self.worksheet)


if __name__ == "__main__":
    unittest.main()
