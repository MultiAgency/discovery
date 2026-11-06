"""Offline regressions for discovery and per-survey release routing."""
import contextlib
import datetime
import io
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("surveys", Path(__file__).with_name("surveys.py"))
surveys = importlib.util.module_from_spec(spec)
spec.loader.exec_module(surveys)
NOW = datetime.datetime(2026, 11, 5, tzinfo=datetime.timezone.utc)
OLD = dict(network="testnet", project="old/discovery", release="1" * 64,
           organizer="service.testnet", locked=True)
NEW = dict(network="testnet", project="new/discovery", release="2" * 64,
           organizer="service.testnet", locked=True, use="surveys")
SURVEY = "a" * 64


def announcement(release=OLD, survey=SURVEY, deadline="2026-11-04T00:00:00Z"):
    fields = {**{k: release[k] for k in surveys.RELEASE_FIELDS}, "survey": survey, "deadline": deadline}
    return "### Confidential survey: Test\n\n```discovery-survey\n" + "\n".join(f"{k}: {v}" for k, v in fields.items()) + "\n```\n"


def report(survey=SURVEY, release=OLD, body="Permitted findings."):
    tx = "3" * 44
    return (f"## Survey report: Test\n\n{body}\n"
            f"\n---\nReleased by survey `{survey}` (finalize tx `{tx}`, release `{release['release']}`). "
            f"Check it with the survey's pinned release settings: `discovery report {survey} --finalize-tx {tx}`; "
            "this verifies the report against the finalize transaction's on-chain output.\n")


def comment(body, user=surveys.BOT):
    return {"body": body, "user": {"login": user}}


class SurveysTests(unittest.TestCase):
    def pending(self, issues, comments, entries=None):
        def pages(endpoint):
            if "/comments?" in endpoint:
                return comments[int(endpoint.split("/issues/")[1].split("/")[0])]
            self.assertEqual(endpoint, "repos/o/r/issues?state=all&per_page=100")
            return issues
        with patch.object(surveys, "pages", side_effect=pages):
            return surveys.pending("o/r", entries or [OLD, NEW], NOW)

    def test_closed_issues_and_prs_keep_original_release(self):
        result = self.pending(
            [{"number": 1, "state": "closed"}, {"number": 2, "pull_request": {"url": "pr"}}],
            {1: [comment(announcement())], 2: [comment(announcement(NEW, "b" * 64))]})
        self.assertEqual([r["number"] for r in result], [1, 2])
        self.assertEqual([r["project"] for r in result], [OLD["project"], NEW["project"]])

    def test_forged_announcements_and_report_markers_are_ignored(self):
        result = self.pending([{"number": 1}], {1: [
            comment(announcement(NEW, "b" * 64), "attacker"),
            comment(announcement()),
            comment(f"Released by survey `{SURVEY}`", "attacker")]})
        self.assertEqual(len(result), 1)
        result = self.pending([{"number": 1}], {1: [comment(announcement()),
            comment(report())]})
        self.assertEqual(result, [])

    def test_model_report_text_cannot_schedule_or_suppress_other_surveys(self):
        quoted = report("b" * 64, body=announcement())
        self.assertEqual(self.pending([{"number": 1}], {1: [comment(quoted)]}), [])
        forged = report("b" * 64, body=f"Released by survey `{SURVEY}`\n" + report())
        result = self.pending([{"number": 1}], {1: [comment(announcement()), comment(forged)]})
        self.assertEqual([item["survey"] for item in result], [SURVEY])

    def test_only_a_complete_terminal_footer_for_the_same_release_completes_a_survey(self):
        for text in [f"Released by survey `{SURVEY}`", report().rstrip() + "\nextra text", report(release=NEW)]:
            result = self.pending([{"number": 1}], {1: [comment(announcement()), comment(text)]})
            self.assertEqual(len(result), 1)

    def test_unknown_release_invalid_deadline_future_and_duplicates(self):
        unknown = {**OLD, "project": "attacker/project"}
        result = self.pending([{"number": 1}], {1: [comment(body) for body in [
            announcement(unknown), announcement(deadline="bad"),
            announcement(deadline="2026-11-04T00:00:00"),
            announcement(deadline="2027-01-01T00:00:00Z"),
            announcement(survey="../../bad"), announcement(), announcement()]]})
        self.assertEqual(len(result), 1)

    def test_github_pages_are_fully_flattened(self):
        with patch.object(surveys, "gh", return_value=[[{"number": 1}], [{"number": 2}]]) as gh:
            self.assertEqual(surveys.pages("endpoint"), [{"number": 1}, {"number": 2}])
            gh.assert_called_once_with("api", "--paginate", "--slurp", "endpoint")

    def test_creation_default_ignores_historical_entries_without_use(self):
        with patch.object(surveys.Path, "read_text", return_value=json.dumps({"releases": [OLD, NEW]})):
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                surveys.release()
            self.assertIn(f"DISCOVERY_PROJECT={NEW['project']}", out.getvalue())
            self.assertNotIn(OLD["project"], out.getvalue())

    def test_creation_without_an_active_release_does_not_fall_back_to_history(self):
        with patch.object(surveys.Path, "read_text", return_value=json.dumps({"releases": [OLD]})):
            with self.assertRaisesRegex(SystemExit, "exactly one entry"):
                surveys.release()

    def test_finalizer_rejects_unknown_release_before_running_commands(self):
        item = {"number": 1, "survey": SURVEY, **OLD, "project": "unknown/discovery"}
        def read(path):
            return json.dumps({"releases": [OLD, NEW]} if str(path) == "releases.json" else [item])
        with patch.object(surveys.Path, "read_text", autospec=True, side_effect=read), \
                patch.object(surveys.subprocess, "run") as run, \
                contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                surveys.finalize("o/r", "due.json")
            run.assert_not_called()

    def test_finalizer_routes_each_survey_to_its_release(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp)
            (path / "releases.json").write_text(json.dumps({"releases": [OLD, NEW]}))
            (path / "due.json").write_text(json.dumps([
                {"number": i, "survey": str(i) * 64, **r} for i, r in enumerate([OLD, NEW], 1)]))
            previous = os.getcwd()
            try:
                os.chdir(path)
                with patch.object(surveys.subprocess, "run") as run:
                    surveys.finalize("o/r", "due.json")
                    calls = run.call_args_list
                    self.assertEqual(len(calls), 4)
                    self.assertEqual(calls[0].kwargs["env"]["DISCOVERY_PROJECT"], OLD["project"])
                    self.assertEqual(calls[2].kwargs["env"]["DISCOVERY_PROJECT"], NEW["project"])
                    self.assertEqual(calls[1].args[0][3], "1")
                    self.assertEqual(calls[3].args[0][3], "2")
            finally:
                os.chdir(previous)


if __name__ == "__main__":
    unittest.main()
