"""Prepare the repo's own issue-enrichment demo, without creating a survey.

All output is public setup material. No respondent answers or credentials belong
here. Live creation remains gated on the approved release and workflow secrets.
"""
import argparse
import datetime
import json
from pathlib import Path
import re
import sys


def definition(repo, issue, hours, now=None):
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        raise ValueError("repo must be owner/name")
    if issue < 1 or not 1 <= hours <= 168:
        raise ValueError("issue must be positive and collection must last 1–168 hours")
    now = now or datetime.datetime.now(datetime.timezone.utc)
    deadline = now + datetime.timedelta(hours=hours)
    epoch = datetime.datetime(1970, 1, 1, tzinfo=datetime.timezone.utc)
    delta = deadline - epoch
    deadline_ns = (delta.days * 86400 + delta.seconds) * 1_000_000_000 + delta.microseconds * 1000
    return {
        "version": 1,
        "kind": "issue_survey",
        "title": "Joining a Discovery survey: what gets in the way?",
        "target": {"repo": repo, "number": issue},
        "deadline_ns": deadline_ns,
        "questions": [
            {"id": "workflow", "prompt": "What were you or your agent trying to accomplish when preparing to join a survey? Describe your actual experience, or say if you have not tried it."},
            {"id": "obstacle", "prompt": "What was difficult, unclear, or error-prone? What did you try, and what happened? Distinguish observations from assumptions."},
            {"id": "outcome", "prompt": "What would joining a survey need to make possible for you, and what verification or disclosure control must it preserve?"},
        ],
        "rules": {"no_direct_quotes": True, "withheld_terms": []},
    }


def live_blockers(root):
    entries = json.loads((root / "releases.json").read_text())["releases"]
    active = [entry for entry in entries if entry.get("use") == "surveys"]
    policy = json.loads((root / "crates/module/workload-policy.json").read_text())
    blockers = []
    if not policy.get("approved_workloads"):
        blockers.append("The confidential model workload has not been approved. See docs/workload-approval.md.")
    if len(active) != 1 or active[0].get("locked") is not True:
        blockers.append("The release registry must contain exactly one locked active surveys release.")
    return blockers


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", default="MultiAgency/discovery")
    parser.add_argument("--issue", type=int, default=1)
    parser.add_argument("--hours", type=int, default=24)
    parser.add_argument("--out", type=Path, default=Path("/tmp/discovery-demo"))
    parser.add_argument("--require-live", action="store_true")
    args = parser.parse_args()
    request = definition(args.repo, args.issue, args.hours)
    blockers = live_blockers(Path(__file__).resolve().parents[2])
    args.out.mkdir(parents=True, exist_ok=True)
    data = json.dumps(request, indent=2) + "\n"
    (args.out / "definition.json").write_text(data)
    (args.out / "request.md").write_text("/survey\n\n```json\n" + data + "```\n")
    status = "# Discovery's own-repo demo\n\n"
    status += f"Target: https://github.com/{args.repo}/issues/{args.issue}\n\n"
    status += "The survey definition is prepared, not published. Its deadline is calculated when this command runs; regenerate before live creation.\n\n"
    if blockers:
        status += "## Live survey blocked\n\n" + "\n".join(f"- {item}" for item in blockers) + "\n\n"
    else:
        status += "Source readiness checks pass. Operators must still verify the registered deployment and configure repository secrets before creating a survey.\n\n"
    status += "The walkthrough uses synthetic responses, in-memory storage, and a synthetic model. It exercises protocol and publication checks; it is not a confidential deployment or a real community finding. Never upload personal answers to Actions, artifacts, or issue comments.\n"
    (args.out / "status.md").write_text(status)
    print(status)
    if args.require_live and blockers:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
