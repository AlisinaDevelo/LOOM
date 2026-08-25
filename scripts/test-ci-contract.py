#!/usr/bin/env python3
"""Verify that local and hosted checks cover the public release contract."""

from __future__ import annotations

import json
import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CI = (ROOT / ".github" / "workflows" / "ci.yml").read_text()
PACKAGE = json.loads((ROOT / "package.json").read_text())
DEPENDENCY_REVIEW = (ROOT / ".github" / "workflows" / "dependency-review.yml").read_text()
DEPENDABOT = (ROOT / ".github" / "dependabot.yml").read_text()
MAKEFILE = (ROOT / "Makefile").read_text()
SECURITY = (ROOT / "scripts" / "security-check.sh").read_text()
DEVICE = (ROOT / "scripts" / "verify-device.sh").read_text()


class CiContractTests(unittest.TestCase):
    def test_frontend_check_runs_the_browser_connector_contract(self) -> None:
        self.assertIn("npm run test:browser-extension", PACKAGE["scripts"]["test"])

    def test_ci_jobs_cover_the_supported_release_paths(self) -> None:
        for job in ("roadmap:", "rust-core:", "rust-msrv:", "rust-advisories:", "frontend:", "tauri-macos:"):
            self.assertIn(job, CI)
        for command in (
            "cargo fmt --all --check",
            "cargo clippy",
            "cargo test",
            "cargo +1.88.0 check",
            "npm ci",
            "npm run check",
            "npm run tauri build -- --debug --no-bundle",
        ):
            self.assertIn(command, CI)

    def test_supply_chain_automation_is_pinned_and_scoped(self) -> None:
        self.assertRegex(
            DEPENDENCY_REVIEW,
            r"actions/dependency-review-action@[0-9a-f]{40}",
        )
        for ecosystem in ("cargo", "npm", "github-actions"):
            self.assertRegex(DEPENDABOT, rf"package-ecosystem:\s+{re.escape(ecosystem)}")

    def test_local_release_hygiene_matches_the_public_contract(self) -> None:
        for marker in ("gitleaks detect", "npm audit --audit-level=high", "cargo metadata --locked"):
            self.assertIn(marker, SECURITY)
        for marker in (
            "run_step fmt cargo fmt --all --check",
            "run_step diff-check git diff --check",
            "run_step ci-contract python3 scripts/test-ci-contract.py",
            "run_step security-check bash scripts/security-check.sh",
        ):
            self.assertIn(marker, DEVICE)
        self.assertIn("python3 scripts/test-ci-contract.py", MAKEFILE)


if __name__ == "__main__":
    unittest.main()
