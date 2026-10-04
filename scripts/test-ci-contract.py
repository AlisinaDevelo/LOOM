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
NPM_LOCK = json.loads((ROOT / "package-lock.json").read_text())
DEPENDENCY_REVIEW = (ROOT / ".github" / "workflows" / "dependency-review.yml").read_text()
DEPENDABOT = (ROOT / ".github" / "dependabot.yml").read_text()
MAKEFILE = (ROOT / "Makefile").read_text()
SECURITY = (ROOT / "scripts" / "security-check.sh").read_text()
DEVICE = (ROOT / "scripts" / "verify-device.sh").read_text()
CARGO = (ROOT / "Cargo.toml").read_text()
DESKTOP = (ROOT / "src-tauri" / "Cargo.toml").read_text()
LOCK = (ROOT / "Cargo.lock").read_text()
TAURI_CEILINGS = {
    "tauri": (2, 12),
    "tauri-build": (2, 7),
    "tauri-codegen": (2, 7),
    "tauri-macros": (2, 7),
    "tauri-plugin": (2, 7),
    "tauri-plugin-dialog": (2, 8),
    "tauri-plugin-fs": (2, 6),
    "tauri-runtime": (2, 12),
    "tauri-runtime-wry": (2, 12),
    "tauri-utils": (2, 10),
}


class CiContractTests(unittest.TestCase):
    def test_desktop_dependency_ranges_preserve_the_declared_msrv(self) -> None:
        # Remove these temporary ceilings only with the policy decision in #329.
        self.assertRegex(CARGO, r'(?m)^rust-version = "1\.88"$')
        self.assertRegex(DESKTOP, r'(?m)^tauri = \{ version = "~2\.11\.\d+",')
        self.assertRegex(DESKTOP, r'(?m)^tauri-build = \{ version = "~2\.6\.\d+",')
        self.assertRegex(DESKTOP, r'(?m)^tauri-plugin-dialog = "~2\.7\.\d+"$')

    def test_locked_desktop_stack_stays_on_the_approved_release_lines(self) -> None:
        seen = set()
        packages = re.findall(r'^\[\[package\]\]\nname = "([^"]+)"\nversion = "([^"]+)"', LOCK, re.M)
        for name, version in packages:
            if name in TAURI_CEILINGS:
                seen.add(name)
                release = tuple(int(part) for part in version.split(".")[:2])
                self.assertLess(release, TAURI_CEILINGS[name], name)
        self.assertEqual(seen, set(TAURI_CEILINGS))

    def assert_npm_tauri_release_lines(self, packages) -> None:
        seen = set()
        for path, package in packages.items():
            if path == "node_modules/@tauri-apps/api" or path.startswith("node_modules/@tauri-apps/cli"):
                seen.add(path)
                self.assertRegex(package["version"], r"^2\.11\.\d+$", path)
        self.assertIn("node_modules/@tauri-apps/api", seen)
        self.assertIn("node_modules/@tauri-apps/cli", seen)

    def test_npm_tauri_ranges_and_lock_match_the_desktop_release(self) -> None:
        self.assertRegex(PACKAGE["dependencies"]["@tauri-apps/api"], r"^~2\.11\.\d+$")
        self.assertRegex(PACKAGE["devDependencies"]["@tauri-apps/cli"], r"^~2\.11\.\d+$")
        self.assert_npm_tauri_release_lines(NPM_LOCK["packages"])

    def test_npm_guard_rejects_an_incompatible_release(self) -> None:
        fixture = {
            "node_modules/@tauri-apps/api": {"version": "2.12.1"},
            "node_modules/@tauri-apps/cli": {"version": "2.11.5"},
        }
        with self.assertRaisesRegex(AssertionError, "2.12.1"):
            self.assert_npm_tauri_release_lines(fixture)

    def test_dependabot_defers_the_blocked_desktop_release_family(self) -> None:
        cargo_section = DEPENDABOT.split("package-ecosystem: cargo", 1)[1].split("package-ecosystem: npm", 1)[0]
        for name, ceiling in TAURI_CEILINGS.items():
            version = ".".join(str(part) for part in ceiling)
            self.assertRegex(cargo_section, rf'dependency-name: {re.escape(name)}\n\s+versions: \[">=\s*{re.escape(version)}"\]')
        npm_section = DEPENDABOT.split("package-ecosystem: npm", 1)[1].split("package-ecosystem: github-actions", 1)[0]
        for name in ("api", "cli"):
            self.assertRegex(npm_section, rf'dependency-name: "@tauri-apps/{name}"\n\s+versions: \[">=\s*2\.12"\]')

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

    def test_device_runner_validates_the_complete_roadmap(self) -> None:
        self.assertIn(
            "run_step roadmap-validate python3 scripts/roadmap.py --validate-only", DEVICE
        )

    def test_device_runner_executes_the_python_suite(self) -> None:
        self.assertIn(
            "run_step python-contract python3 -m unittest discover -s tests -v", DEVICE
        )

    def test_device_runner_executes_the_browser_protocol_contract(self) -> None:
        self.assertIn(
            "run_step browser-protocol python3 scripts/test-browser-capture-protocol.py", DEVICE
        )

    def test_roadmap_recipe_covers_the_protocol_and_python_suite(self) -> None:
        recipe = MAKEFILE.split("roadmap-check:", 1)[1].split("\n\n", 1)[0]
        for command in (
            "python3 scripts/roadmap.py --validate-only",
            "python3 -m unittest discover -s tests -v",
            "python3 scripts/test-browser-capture-protocol.py",
        ):
            self.assertIn(command, recipe)
        self.assertIn("run: make roadmap-check", CI)

    def test_local_release_hygiene_matches_the_public_contract(self) -> None:
        for marker in (
            "gitleaks detect", "npm audit --audit-level=high", "cargo metadata --locked",
            "AUDIT_ARGS=(audit --file Cargo.lock --deny warnings)",
        ):
            self.assertIn(marker, SECURITY)
        for marker in (
            "run_step fmt cargo fmt --all --check",
            "run_step diff-check git diff --check",
            "run_step ci-contract python3 scripts/test-ci-contract.py",
            "run_step native-host-build cargo build --locked -q -p loom --bin loom-native-host",
            "run_step native-host-contract python3 scripts/test-native-host.py --host target/debug/loom-native-host",
            "run_step security-check bash scripts/security-check.sh",
        ):
            self.assertIn(marker, DEVICE)
        self.assertIn("python3 scripts/test-ci-contract.py", MAKEFILE)
        self.assertIn("run_step background-jobs python3 scripts/test-background-jobs.py", DEVICE)
        self.assertIn("python3 scripts/test-background-jobs.py --loom target/debug/loom", CI)
        self.assertIn("run_step queued-indexing python3 scripts/test-queued-indexing.py", DEVICE)
        self.assertIn("python3 scripts/test-queued-indexing.py --loom target/debug/loom", CI)

    def test_real_extractor_is_built_tested_and_staged_beside_the_cli(self) -> None:
        self.assertIn("cargo build --locked -p loom-extraction --bin loom-extractor", CI)
        self.assertEqual(CI.count("cargo build --locked -p loom-extraction --bin loom-extractor"), 2)
        self.assertIn("run_step rust-workspace-helper cargo build --locked -p loom-extraction --bin loom-extractor", DEVICE)
        self.assertIn("cargo test --locked -p loom-core -p loom-cli -p loom-extraction", CI)
        self.assertIn("run_step rust-msrv-helper cargo +1.88.0 build --locked -p loom-extraction", DEVICE)
        self.assertIn("cargo build --locked -q -p loom-cli -p loom-extraction --bins", DEVICE)
        self.assertIn('cp "$ROOT/target/debug/loom-extractor" "$EVIDENCE_DIR/loom-extractor"', DEVICE)
        self.assertIn('run_step queued-responsiveness env LOOM_TEST_RESPONSE_REPORT=', DEVICE)
        self.assertIn(
            'cargo test --locked -p loom-core jobs::tests::persistent_retrieval_during_native_queued_ocr',
            DEVICE,
        )
        self.assertNotIn(
            'cargo test --locked -p loom-core --lib jobs::tests::persistent_retrieval_during_native_queued_ocr',
            DEVICE,
        )
        self.assertIn('jobs::tests::persistent_retrieval_during_native_queued_ocr -- --exact --nocapture', DEVICE)


if __name__ == "__main__":
    unittest.main()
