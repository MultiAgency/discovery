import datetime
import json
from pathlib import Path
import tempfile
import unittest

import prepare_demo


class DemoTests(unittest.TestCase):
    def test_definition_binds_own_issue_and_fresh_deadline(self):
        now = datetime.datetime(2026, 11, 5, 12, tzinfo=datetime.timezone.utc)
        result = prepare_demo.definition("MultiAgency/discovery", 1, 24, now)
        self.assertEqual(result["target"], {"repo": "MultiAgency/discovery", "number": 1})
        self.assertEqual(result["deadline_ns"], int(now.timestamp() + 86400) * 1_000_000_000)
        self.assertEqual(result["kind"], "issue_survey")
        self.assertTrue(result["rules"]["no_direct_quotes"])

    def test_rejects_invalid_target_and_collection_window(self):
        for repo, issue, hours in [("bad/repo/path", 1, 24), ("o/r", 0, 24), ("o/r", 1, 0), ("o/r", 1, 169)]:
            with self.assertRaises(ValueError):
                prepare_demo.definition(repo, issue, hours)

    def test_live_gate_refuses_historical_release_and_empty_approval(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "crates/module").mkdir(parents=True)
            (root / "releases.json").write_text(json.dumps({"releases": [{"locked": True}]}))
            (root / "crates/module/workload-policy.json").write_text(json.dumps({"approved_workloads": []}))
            self.assertEqual(len(prepare_demo.live_blockers(root)), 2)


if __name__ == "__main__":
    unittest.main()
