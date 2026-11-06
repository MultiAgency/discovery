# Discovery researches its own onboarding

The demo belongs in this repository. Its first research topic is
[issue #1: Joining a survey takes too many manual steps](https://github.com/MultiAgency/discovery/issues/1).
The service should improve its own requirements using evidence from people and
agents trying to contribute. The demo ends with a shared report and a
maintainer-reviewed refinement to the existing issue's requirements.

## Try the protocol now

Open [the own-repo demo Action](https://github.com/MultiAgency/discovery/actions/workflows/demo.yml),
select **Run workflow**, and keep **walkthrough** selected. The Action prepares
a survey for issue #1, runs the local request-to-report acceptance walkthrough,
and puts the resulting synthetic report and readiness status in its summary.
Public setup files and synthetic output are available as a run artifact. It
does not create a live survey, collect responses, or post research findings.

From a checkout, the same walkthrough is:

```bash
cargo test --locked -p discovery-module finalize::tests::request_to_report_demo -- --exact --nocapture
python3 .github/scripts/prepare_demo.py --out /tmp/discovery-demo
```

The protocol uses real definition hashing, per-survey signatures, response
encryption, deadline checks, publication checks, and cleanup. Storage is in
memory and the model response is synthetic. The fixture is about first-run CLI
configuration; it illustrates the lifecycle, not actual findings about issue #1.

## Activate the real survey

The current source deliberately blocks live creation: no confidential model
workload has been approved, no new active locked release is registered, and
the repository's local organizer key (`DISCOVERY_ORGANIZER_KEY`) and shared
submission key (`DISCOVERY_PAYMENT_KEY`) must be configured. Follow
[workload approval](workload-approval.md) and [operations](operations.md).
Do not bypass those gates or use the historical release to imply this source
was verified.

Once those steps are complete, run the Action in **live** mode against issue #1.
It generates a fresh deadline, creates the survey through the CLI, and posts
the real machine-readable announcement. The scheduled finalize Action later
publishes the permitted report. The live path is implemented but has not yet
been verified end to end for this source.

The initial questions are:

1. What were you or your agent trying to accomplish when preparing to join a
   survey? Describe actual experience, or say you have not tried it.
2. What was difficult, unclear, or error-prone? What did you try, and what
   happened? Distinguish observations from assumptions.
3. What would joining need to make possible, and what verification or disclosure
   control must it preserve?

The questions invite alternatives to the issue's proposed one-command solution.
Direct quotes are disabled. Collection ends when finalization begins; its
deadline is the earliest finalization time. Retention and privacy limits follow
[the service policy](decisions.md).

## Invitation for teammates — use after the live announcement appears

Give your existing agent this prompt, replacing the announcement link:

> Help me respond to Discovery's onboarding survey at <announcement link>.
> Read this repository's `skills/discovery-respondent/SKILL.md` and its
> references. Verify the survey against this checkout's release registry.
> Use only context I have authorized. Ask me about missing details through our
> usual conversation. Show me the answers, evidence basis, and publication
> permissions before submission. Do not post my answers or announce my
> participation on GitHub. If there is no verified open survey, stop rather
> than submitting elsewhere.

Build the CLI with `cargo build --locked -p discovery-cli`; its executable is
`target/debug/discovery`. A respondent needs the repo/skill, an existing agent,
and the verified announcement, not an individual wallet or model credential.
Do not submit answers as Action inputs, artifacts, comments, or PRs.

## The visible payoff

Before collection, issue #1 proposes a one-command join and locally pinned
releases. After release, the maintainer's agent verifies the report, states
which assumptions the evidence supports or challenges, and proposes revisions
to the issue's acceptance criteria. The maintainer approves those revisions.

Success is one defensible requirement change or corrected assumption, with
restricted identifying material withheld. A successful synthetic workflow is
not evidence of confidential processing or community participation.
