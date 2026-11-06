"""GitHub glue for the survey Actions. The CLI does the survey work; this only
reads announcements and invokes the CLI.

  surveys.py definition <comment.md>  -> prints the ```json block of a /survey comment
  surveys.py release                  -> prints KEY=VALUE lines for the surveys release
  surveys.py due <repo>               -> JSON pending issue surveys, including closed issues
  surveys.py finalize <repo> <due.json> -> finalize and post each survey on its original release
"""
import datetime
import json
import os
from pathlib import Path
import tempfile
import re
import subprocess
import sys

FENCE = re.compile(r"```(\w[\w-]*)\n(.*?)```", re.S)


def blocks(text, lang):
    return [body for kind, body in FENCE.findall(text) if kind == lang]


def definition(path):
    found = blocks(Path(path).read_text(), "json")
    if len(found) != 1:
        sys.exit("a /survey comment needs exactly one ```json block with the survey definition")
    print(found[0])


def release():
    entries = [e for e in json.loads(Path("releases.json").read_text())["releases"] if e.get("use") == "surveys"]
    if len(entries) != 1:
        sys.exit("releases.json needs exactly one entry with \"use\": \"surveys\"")
    e = entries[0]
    if e.get("legacy_format"):
        sys.exit("the active release uses an unsupported historical report format")
    for key, field in [("NETWORK", "network"), ("PROJECT", "project"), ("RELEASE", "release"), ("ORGANIZER", "organizer")]:
        print(f"DISCOVERY_{key}={e[field]}")


def gh(*args):
    return json.loads(subprocess.run(["gh", *args], check=True, capture_output=True, text=True).stdout)


RELEASE_FIELDS = ("network", "project", "release", "organizer")
BOT = "github-actions[bot]"
REPORT_FOOTER = re.compile(
    r"Released by survey `(?P<survey>[0-9a-f]{64})` "
    r"\(finalize tx `(?P<tx>[1-9A-HJ-NP-Za-km-z]+)`, release `(?P<release>[0-9a-f]{64})`\)\. "
    r"Check it with the survey's pinned release settings: "
    r"`discovery report (?P=survey) --finalize-tx (?P=tx)`; "
    r"this verifies the report against the finalize transaction's on-chain output\.\n?"
)


def released_report(text):
    # Only the CLI's terminal provenance footer is control data. Report prose
    # is model output, even though the same bot publishes both documents.
    if not text.startswith("## Survey report: "):
        return None
    _, separator, footer = text.rpartition("\n---\n")
    match = REPORT_FOOTER.fullmatch(footer) if separator else None
    return (match["survey"], match["release"]) if match else None


def trusted_release(fields, entries):
    return next((e for e in entries if e.get("locked") is True
                 and all(fields.get(k) == e.get(k) for k in RELEASE_FIELDS)), None)


def pages(endpoint):
    # The REST endpoint includes pull requests; pending() excludes them.
    return [item for page in gh("api", "--paginate", "--slurp", endpoint) for item in page]


def pending(repo, entries, now):
    result, seen = [], set()
    for issue in pages(f"repos/{repo}/issues?state=all&per_page=100"):
        if "pull_request" in issue:
            continue
        n = issue["number"]
        comments = pages(f"repos/{repo}/issues/{n}/comments?per_page=100")
        # Public comments are untrusted: only our Actions' announcements and
        # report markers can schedule or suppress privileged finalization.
        texts = [c["body"] for c in comments if c.get("user", {}).get("login") == BOT]
        reported = {released_report(t) for t in texts}
        for text in texts:
            # A report can quote an announcement. Only announcement documents
            # may schedule work; never search every bot document for commands.
            if not text.startswith("### Confidential survey: "):
                continue
            announcements = blocks(text, "discovery-survey")
            if len(announcements) != 1:
                continue
            for block in announcements:
                fields = dict(line.split(": ", 1) for line in block.strip().splitlines() if ": " in line)
                survey, deadline = fields.get("survey", ""), fields.get("deadline", "")
                pinned = trusted_release(fields, entries)
                if not pinned or pinned.get("legacy_format") or not re.fullmatch(r"[0-9a-f]{64}", survey):
                    continue
                try:
                    due_at = datetime.datetime.fromisoformat(deadline.replace("Z", "+00:00"))
                    if due_at.tzinfo is None or due_at > now:
                        continue
                except ValueError:
                    continue
                identity = tuple(pinned[k] for k in RELEASE_FIELDS) + (survey,)
                if (survey, pinned["release"]) not in reported and identity not in seen:
                    seen.add(identity)
                    result.append({"number": n, "survey": survey,
                                   **{k: pinned[k] for k in RELEASE_FIELDS}})
    return result


def due(repo):
    entries = json.loads(Path("releases.json").read_text())["releases"]
    print(json.dumps(pending(repo, entries, datetime.datetime.now(datetime.timezone.utc))))


def finalize(repo, path):
    entries = json.loads(Path("releases.json").read_text())["releases"]
    failed = False
    for item in json.loads(Path(path).read_text()):
        try:
            pinned = trusted_release(item, entries)
            if not pinned or pinned.get("legacy_format") or not re.fullmatch(r"[0-9a-f]{64}", item["survey"]):
                raise ValueError("pending survey is not on a trusted release")
            number = int(item["number"])
            if number <= 0:
                raise ValueError("invalid issue or PR number")
            env = os.environ.copy()
            env.update({f"DISCOVERY_{k.upper()}": pinned[k] for k in RELEASE_FIELDS})
            with tempfile.TemporaryDirectory(prefix="discovery-finalize-") as temp:
                comment = str(Path(temp) / "report.md")
                subprocess.run(["target/release/discovery", "organizer", "finalize", item["survey"],
                                "--comment", comment], env=env, check=True)
                subprocess.run(["gh", "issue", "comment", str(number), "--repo", repo,
                                "--body-file", comment], check=True)
        except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
            print(f"finalize failed for {item.get('survey', '?')}: {error}", file=sys.stderr)
            failed = True
    if failed:
        sys.exit(1)


if __name__ == "__main__":
    command = sys.argv[1] if len(sys.argv) > 1 else ""
    {"definition": lambda: definition(sys.argv[2]), "release": release, "due": lambda: due(sys.argv[2]),
     "finalize": lambda: finalize(sys.argv[2], sys.argv[3])}.get(
        command, lambda: sys.exit(__doc__)
    )()
