from __future__ import annotations

import copy
import importlib.util
import json
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("loom_roadmap", ROOT / "scripts" / "roadmap.py")
assert SPEC and SPEC.loader
roadmap = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = roadmap
SPEC.loader.exec_module(roadmap)


class RoadmapTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.manifest = roadmap.load_manifest(ROOT / "roadmap" / "roadmap.json")
        cls.milestone_by_key = {item["key"]: item for item in cls.manifest["milestones"]}

    def matching_snapshot(self) -> roadmap.Snapshot:
        specs = roadmap.label_specs(self.manifest)
        labels = [{"name": name, "color": color, "description": description} for name, (color, description) in specs.items()]
        milestones = []
        milestone_number = {}
        for number, item in enumerate(self.manifest["milestones"], start=1):
            milestone_number[item["key"]] = number
            milestones.append({
                "number": number, "title": item["title"],
                "description": f"{item['focus']} Exit gate: {item['exit_criteria']}",
                "due_on": f"{item['due_on']}T23:59:59Z", "state": "open",
            })
        issues = []
        issue_by_task = {}
        numeric_by_task = {}
        for number, item in enumerate(self.manifest["issues"], start=1):
            numeric = 10_000 + number
            numeric_by_task[item["id"]] = numeric
            live = {
                "id": numeric, "number": number, "title": item["title"],
                "body": roadmap.public_body(item, self.milestone_by_key[item["quarter"]]),
                "labels": [{"name": name} for name in item["labels"]],
                "state": "closed" if item["status"] == "done" else "open",
                "milestone": {"number": milestone_number[item["quarter"]]},
            }
            issues.append(live)
            issue_by_task[item["id"]] = live
        for offset, item in enumerate(self.manifest["retired_issues"], start=1):
            number = len(issues) + 1
            numeric = 20_000 + offset
            live = {
                "id": numeric, "number": number,
                "title": f"Consolidated roadmap item {item['id']} into {item['merged_into']}",
                "body": roadmap.retired_body(item),
                "labels": [{"name": "status:retired"}],
                "state": "closed", "state_reason": "not_planned", "milestone": None,
            }
            issues.append(live)
            issue_by_task[item["id"]] = live
        subissues = {}
        blocked_by = {}
        for item in self.manifest["issues"]:
            if item["parent"] is None:
                subissues[item["id"]] = {
                    numeric_by_task[child["id"]]
                    for child in self.manifest["issues"]
                    if child["parent"] == item["id"]
                }
            blocked_by[item["id"]] = {numeric_by_task[blocker] for blocker in item["depends_on"]}
        return roadmap.Snapshot(labels, milestones, issues, issue_by_task, subissues, blocked_by)

    def test_program_has_five_year_shape(self) -> None:
        self.assertEqual(154, len(self.manifest["issues"]))
        self.assertEqual(4, len(self.manifest["retired_issues"]))
        self.assertEqual(20, len(self.manifest["milestones"]))
        self.assertEqual(13, len({item["phase"] for item in self.manifest["issues"]}))
        self.assertEqual(141, sum(item["parent"] is not None for item in self.manifest["issues"]))
        self.assertEqual(314, sum(len(item["depends_on"]) for item in self.manifest["issues"]))
        self.assertEqual("2026-08-23", self.manifest["program"]["starts_on"])
        self.assertEqual("2031-08-23", self.manifest["program"]["ends_on"])

    def test_public_mutations_use_the_slow_single_writer_interval(self) -> None:
        self.assertGreaterEqual(roadmap.WRITE_DELAY_SECONDS, 8.0)

    def test_every_issue_has_execution_metadata(self) -> None:
        for issue in self.manifest["issues"]:
            with self.subTest(issue=issue["id"]):
                self.assertRegex(issue["id"], r"^\d{4}$")
                self.assertTrue(issue["outcome"])
                self.assertGreaterEqual(len(issue["acceptance_criteria"]), 2)
                self.assertLessEqual(len(issue["acceptance_criteria"]), 4)
                self.assertEqual(1, sum(label.startswith("type:") for label in issue["labels"]))
                self.assertEqual(1, sum(label.startswith("priority:") for label in issue["labels"]))
                self.assertGreaterEqual(sum(label.startswith("area:") for label in issue["labels"]), 1)
                self.assertIn(issue["quarter"], self.milestone_by_key)

    def test_manifest_is_public_product_metadata_only(self) -> None:
        text = json.dumps(self.manifest).lower()
        for forbidden in ("gpt-", ".forge/", "/users/", "agent:", "model:"):
            self.assertNotIn(forbidden, text)

    def test_public_issue_body_is_complete_and_sanitized(self) -> None:
        issue = next(item for item in self.manifest["issues"] if item["id"] == "0701")
        body = roadmap.public_body(issue, self.milestone_by_key[issue["quarter"]])
        self.assertIn("<!-- loom-roadmap:v2 id=0701 -->", body)
        for heading in ("## Outcome", "## Acceptance criteria", "## Prerequisites", "## Closure evidence", "## Product boundary"):
            self.assertIn(heading, body)
        self.assertIn("Quarter: **Q13**", body)
        self.assertNotIn("gpt-", body.lower())
        self.assertNotIn(".forge", body.lower())

    def test_duplicate_live_marker_fails_before_mutation(self) -> None:
        issues = [
            {"number": 1, "body": "<!-- loom-roadmap:v1 id=0001 -->", "user": {"login": "AlisinaDevelo"}},
            {"number": 2, "body": "<!-- forge-task:v1 id=0001 sync=abc -->", "user": {"login": "AlisinaDevelo"}},
        ]
        with self.assertRaisesRegex(ValueError, "duplicate LOOM roadmap marker"):
            roadmap.index_managed_issues(issues, {"0001"}, "AlisinaDevelo")

    def test_unknown_live_marker_fails_before_mutation(self) -> None:
        with self.assertRaisesRegex(ValueError, "unknown LOOM roadmap marker"):
            roadmap.index_managed_issues(
                [{"number": 1, "body": "<!-- forge-task:v1 id=9999 sync=abc -->", "user": {"login": "AlisinaDevelo"}}],
                {"0001"}, "AlisinaDevelo",
            )

    def test_untrusted_marker_fails_before_mutation(self) -> None:
        issue = {"number": 1, "body": "<!-- loom-roadmap:v2 id=0001 -->", "user": {"login": "someone-else"}}
        with self.assertRaisesRegex(ValueError, "untrusted author"):
            roadmap.index_managed_issues([issue], {"0001"}, "AlisinaDevelo")

    def test_later_quarter_dependency_is_rejected(self) -> None:
        broken = copy.deepcopy(self.manifest)
        by_id = {item["id"]: item for item in broken["issues"]}
        by_id["0100"]["depends_on"] = ["1309"]
        with self.assertRaisesRegex(ValueError, "depends on later-quarter"):
            roadmap.validate_manifest(broken)

    def test_cycle_is_rejected(self) -> None:
        broken = copy.deepcopy(self.manifest)
        by_id = {item["id"]: item for item in broken["issues"]}
        by_id["0100"]["depends_on"] = ["0101"]
        by_id["0101"]["depends_on"] = ["0100"]
        with self.assertRaisesRegex(ValueError, "dependency cycle"):
            roadmap.validate_manifest(broken)

    def test_generic_outcome_is_rejected(self) -> None:
        broken = copy.deepcopy(self.manifest)
        broken["issues"][0]["outcome"] = "This produces a verifiable product boundary."
        with self.assertRaisesRegex(ValueError, "concrete outcome"):
            roadmap.validate_manifest(broken)

    def test_existing_issue_is_patched_in_place(self) -> None:
        desired = self.manifest["issues"][0]
        live = {"id": 501, "number": 77, "title": "stale", "body": roadmap.issue_marker(desired["id"]), "labels": [], "state": "open", "milestone": None}
        snapshot = roadmap.Snapshot(labels=[], milestones=[], issues=[live], issue_by_task={desired["id"]: live})
        plan = roadmap.build_plan(self.manifest, snapshot, include_relationships=False)
        self.assertNotIn(desired["id"], plan["mutations"]["create_issues"])
        # Milestones must reconcile first; identity is still retained and never POSTed as a duplicate.
        self.assertEqual(77, snapshot.issue_by_task[desired["id"]]["number"])

    def test_fully_matching_snapshot_is_idempotent(self) -> None:
        snapshot = self.matching_snapshot()
        plan = roadmap.build_plan(self.manifest, snapshot, include_relationships=True)
        self.assertEqual(0, roadmap.mutation_total(plan))
        verified = roadmap.verify_live(self.manifest, snapshot, plan)
        self.assertEqual(0, verified["mutation_count"])
        self.assertEqual(314, verified["dependency_edges"])

    def test_unexpected_closed_issue_fails_live_verification(self) -> None:
        snapshot = self.matching_snapshot()
        snapshot.issue_by_task["0102"]["state"] = "closed"
        plan = roadmap.build_plan(self.manifest, snapshot, include_relationships=True)
        self.assertEqual(["0102"], plan["warnings"]["unexpected_closed"])
        with self.assertRaisesRegex(ValueError, "unexpected closed issues"):
            roadmap.verify_live(self.manifest, snapshot, plan)

    def test_public_metadata_detector_covers_routing_fields(self) -> None:
        for body in ("agent: planner", "model: local-encoder", "Assigned agent: private", "Assigned model: private"):
            self.assertTrue(roadmap.contains_unsafe_public_metadata(body))

    def test_public_metadata_detector_allows_normal_threat_model_prose(self) -> None:
        self.assertFalse(roadmap.contains_unsafe_public_metadata("The threat model: covers redirects."))


if __name__ == "__main__":
    unittest.main()
