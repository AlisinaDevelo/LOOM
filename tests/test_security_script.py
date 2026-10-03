"""Exercise advisory enforcement using isolated tool stubs, not a simulated security audit."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "security-check.sh"
POLICY = SCRIPT.parents[1] / ".cargo" / "audit.toml"
STRICT_POLICY = '''# Project policy takes precedence over the operator's Cargo audit configuration.
[advisories]
ignore = []
informational_warnings = ["unmaintained", "unsound", "notice"]
severity_threshold = "none"

[database]
url = "https://github.com/RustSec/advisory-db.git"
fetch = true
stale = false

[output]
deny = ["warnings"]
format = "terminal"
quiet = false
show_tree = true

[target]
arch = []
os = []

[yanked]
enabled = true
update_index = true
'''


class LocalSecurityTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="loom-security-contract-")
        self.root = Path(self.directory.name)
        (self.root / "scripts").mkdir()
        shutil.copyfile(SCRIPT, self.root / "scripts" / "security-check.sh")
        (self.root / ".cargo").mkdir()
        shutil.copyfile(POLICY, self.root / ".cargo" / "audit.toml")
        self.bin = self.root / "tools"
        self.bin.mkdir()
        self.calls = self.root / "calls.jsonl"
        for name in ("gitleaks", "npm", "cargo", "audit tool"):
            tool = self.bin / name
            tool.write_text(
                "#!" + shutil.which("python3") + "\n"
                "import json, os, sys\n"
                "with open(os.environ['SECURITY_CALLS'], 'a') as f:\n"
                "    f.write(json.dumps([os.path.basename(sys.argv[0]), *sys.argv[1:]]) + '\\n')\n"
                "name = os.path.basename(sys.argv[0])\n"
                "code = os.environ.get('AUDIT_EXIT', '0') if name == 'audit tool' else "
                "os.environ.get('CHECK_EXIT', '0') if name == os.environ.get('FAIL_TOOL') else '0'\n"
                "sys.exit(int(code))\n"
            )
            tool.chmod(0o755)
        self.environment = {
            "PATH": str(self.bin) + os.pathsep + os.defpath,
            "SECURITY_CALLS": str(self.calls),
            "LOOM_CARGO_AUDIT_BIN": str(self.bin / "audit tool"),
        }

    def tearDown(self):
        self.directory.cleanup()

    def run_script(self):
        return subprocess.run(
            ["/bin/bash", str(self.root / "scripts" / "security-check.sh")],
            env=self.environment, text=True, capture_output=True, check=False,
        )

    def recorded(self):
        return [json.loads(line) for line in self.calls.read_text().splitlines()]

    def test_success_includes_strict_audit_and_all_other_checks(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.recorded()
        self.assertEqual([call[0] for call in calls], ["gitleaks", "npm", "cargo", "audit tool"])
        self.assertEqual(calls[-1], ["audit tool", "audit", "--file", "Cargo.lock", "--deny", "warnings"])
        self.assertIn("--locked", calls[2])
        self.assertIn("--redact", calls[0])

    def test_finding_exit_code_is_not_suppressed(self):
        self.environment["AUDIT_EXIT"] = "1"
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.recorded()[-1][0], "audit tool")

    def test_database_path_is_one_literal_argument(self):
        self.environment["LOOM_CARGO_AUDIT_DB"] = str(self.root / "db with spaces;no-command")
        self.assertEqual(self.run_script().returncode, 0)
        self.assertEqual(self.recorded()[-1][-2:], ["--db", self.environment["LOOM_CARGO_AUDIT_DB"]])

    def test_missing_audit_tool_fails_closed(self):
        self.environment["LOOM_CARGO_AUDIT_BIN"] = str(self.root / "absent")
        result = self.run_script()
        self.assertEqual(result.returncode, 127)
        self.assertIn("cargo-audit is required", result.stderr)
        self.assertFalse(self.calls.exists())

    def test_default_tool_is_required_not_optional(self):
        del self.environment["LOOM_CARGO_AUDIT_BIN"]
        result = self.run_script()
        self.assertEqual(result.returncode, 127)
        self.assertFalse(self.calls.exists())

    def test_tracked_policy_has_no_suppression_or_target_filter(self):
        self.assertEqual(POLICY.read_text(), STRICT_POLICY)

    def test_missing_policy_refuses_fallback_to_user_configuration(self):
        (self.root / ".cargo" / "audit.toml").unlink()
        result = self.run_script()
        self.assertEqual(result.returncode, 2)
        self.assertIn("refusing user defaults", result.stderr)
        self.assertFalse(self.calls.exists())

    def test_each_preceding_check_failure_stops_the_audit(self):
        for name in ("gitleaks", "npm", "cargo"):
            with self.subTest(tool=name):
                if self.calls.exists():
                    self.calls.unlink()
                self.environment.update(FAIL_TOOL=name, CHECK_EXIT="4")
                self.assertEqual(self.run_script().returncode, 4)
                calls = self.recorded()
                self.assertEqual(calls[-1][0], name)
                self.assertNotIn("audit tool", [call[0] for call in calls])

    def test_audit_operational_error_is_not_suppressed(self):
        self.environment["AUDIT_EXIT"] = "2"
        self.assertEqual(self.run_script().returncode, 2)


if __name__ == "__main__":
    unittest.main()
