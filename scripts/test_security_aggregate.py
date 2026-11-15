"""Regression checks for the audit gate's missing-evidence behavior."""

import json
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("security_aggregate.py")


class AggregateTests(unittest.TestCase):
    def run_aggregate(self, omitted=()):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifacts = root / "artifacts"
            for name in ("dependencies", "licenses", "static", "unsafe", "secrets", "sbom", "tests"):
                if name in omitted:
                    continue
                job = artifacts / f"security-{name}"
                job.mkdir(parents=True)
                (job / "security-audit.json").write_text(json.dumps({
                    "commit": "abc123", "checks": [{"name": name, "status": "PASS", "checks": []}]
                }))
            completed = subprocess.run(["python3", str(SCRIPT), str(artifacts)], cwd=root,
                                       capture_output=True, text=True, check=False)
            report = json.loads((root / "security/reports/security-audit.json").read_text())
            return completed.returncode, report

    def test_complete_evidence_passes(self):
        code, report = self.run_aggregate()
        self.assertEqual((code, report["status"]), (0, "PASS"))
        self.assertEqual(next(c for c in report["checks"] if c["name"] == "fuzz")["status"], "SKIPPED")

    def test_missing_job_is_error(self):
        code, report = self.run_aggregate({"secrets"})
        self.assertEqual((code, report["status"]), (2, "ERROR"))
        self.assertEqual(next(c for c in report["checks"] if c["name"] == "secrets")["status"], "ERROR")


if __name__ == "__main__":
    unittest.main()
