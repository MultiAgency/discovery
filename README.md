# Confidential surveys for agents

Maintainers ask for context about an existing issue. Users' personal agents
prepare authorized responses. Confidential processing returns findings that help
the maintainer understand workflows, obstacles and desired outcomes without
receiving individual answers.

Individual responses are research inputs, not issue drafts. The GitHub flow is
**existing issue → survey → private responses → shared report on that issue**.
The service does not create an issue for each response. The maintainer's agent
uses the report to propose refinements to the existing issue's requirements.

[The original November 4 concept](docs/concept.md) is the product specification.
Existing agents supply the text or voice interaction for preparing responses
and interpreting reports.

## Use the MVP

**Start with [Discovery's own-repo demo](docs/demo.md).** Research the onboarding
friction in issue #1, run the synthetic request-to-report walkthrough in Actions,
and follow the live activation steps once the confidential release is approved.

1. A maintainer uses the [organizer skill](skills/discovery-organizer/SKILL.md)
   to prepare and approve an issue survey, then posts `/survey` with its definition.
2. The repository's Action creates the survey and posts a machine-readable announcement.
3. Personal agents follow the [respondent skill](skills/discovery-respondent/SKILL.md):
   verify the survey, clarify the human's experience, get approval, and submit privately.
4. After the deadline, finalization closes collection, synthesizes the batch,
   checks disclosure rules, releases or withholds a report, and erases response contents.
5. The maintainer's existing agent interprets the report alongside other evidence.
   Success means changing an acceptance criterion or correcting an assumption.

Respondents use per-survey local signing keys. A hosted interviewer, frontend,
prioritization engine and usage verification are outside this MVP.

## Build and check locally

Tested with Rust 1.97.0 and Python 3. Install the `wasm32-wasip2` Rust target
for the module build. No credentials are required for these checks.

```bash
cargo build --locked -p discovery-cli
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s .github/scripts -p 'test_*.py'
cargo build --release --locked --target wasm32-wasip2 -p discovery-module
```

The CLI is `target/debug/discovery`; run `--help` for commands. The same checks
run in GitHub Actions on pushes and pull requests.

## Local acceptance walkthrough

```bash
cargo test -p discovery-module request_to_report_demo -- --nocapture
```

This exercises real definition hashing, per-survey signatures, encrypted
submission, deadline checks, publication filtering, structured report output and
cleanup. It uses in-memory host storage and a synthetic model response; it does
not demonstrate deployed attestation, actual model synthesis, or GitHub posting.
The output is a permitted report proposing a concrete first-run acceptance
criterion from synthetic human-confirmed and agent-observed evidence.

## Release readiness

**The current source is not a deployed, approved release.** The embedded model
workload policy is empty and blocks new survey creation and real inference.
The registry retains the historical locked testnet release for existing surveys,
with no active creation entry. New `/survey` requests fail before spending on a
survey until operators add a reviewed, locked release marked `"use": "surveys"`.
Historical hashes have not been replaced.

Follow [workload approval](docs/workload-approval.md), then
[operations](docs/operations.md) to build, review and register the new release.
Before collecting personal data, run the full request-to-report cycle on an
existing issue using synthetic identifying material. Verify the released report,
closure and cleanup, and record what the report changed about the issue. The
historical [spike results](spike/RESULTS.md) are feasibility evidence for older
builds, not verification of this source.

## Boundaries and implementation

Read [the disclosure, timing and retention terms](docs/decisions.md) before
requesting or submitting feedback. Confidentiality is not a guarantee of
anonymity: distinctive stories and public timing can still identify someone.

| Component | Responsibility |
| --- | --- |
| `crates/protocol` | Definitions, response permissions, envelopes, signatures and report format |
| `crates/cli` | Organizer and respondent commands, release and report verification |
| `crates/module` | OutLayer storage, collection, confidential synthesis and publication checks |
| `crates/nearai` | Attestation, text-only encrypted inference, signed turns and receipt evidence |
| `.github` | Issue announcements, scheduled report posting and automated checks |
| `skills` | Existing agents' organizer and respondent workflows |

[Research](research/) records implementation research, not additional product
requirements. The current protocol contains issue surveys and shared research
reports only. Existing locked deployments remain unchanged; use a compatible
earlier CLI for their older report formats, as described in [releases](docs/releases.md).
