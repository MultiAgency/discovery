<!-- TEMPLATE. Not an invitation. `scripts/invitation.sh` fills every {{…}}
from a successful `discovery verify` of an accepting post-lock survey and
removes this notice; never fill it by hand. Before sharing the result: the
synthetic smoke test passed on the registered release, the organizer reviewed
the rendered page, and a teammate followed it from a clean machine without
extra help. -->

# Try Discovery with your agent

We're testing Discovery in a small pilot run by the service organizer. Your
existing agent installs the CLI, verifies the survey, helps you
answer, shows you exactly what it will submit, and submits only what you
approve. Afterwards a synthesized report is posted on
[{{target}}]({{issue_url}}), and supported findings become proposed GitHub
issues.

Survey: **{{title}}**

We want to learn what joining is actually like, including what went wrong. If
you get stuck, that is a finding: tell us through the private help route below,
not in public.

## Before you start

- A machine where you can install software, with `git` and a Rust toolchain
  (`rustup`; tested with Rust 1.97). No wallet, NEAR account or GitHub account is
  needed to respond.
- An agent that can run shell commands and read files, such as Claude Code or
  Codex.
- Time for a Rust build, verification and approving your answers. The first
  verification on a machine checks the release's bootstrap proof and can take
  a few minutes on the free public RPC. We are measuring the complete joining
  time during this pilot.

## Give this to your agent

> Help me respond to the Discovery survey described at {{page_url}}.
> Read that invitation for installation instructions and survey values. Use
> https://github.com/MultiAgency/discovery at pinned commit `{{commit}}`, install
> the CLI from that checkout, and follow `skills/discovery-respondent/SKILL.md`
> from the same commit. Stop if the survey does not verify or the release does
> not match. Use only context I authorize and ask me about missing details. Show me
> every answer, its basis and its publication permission before submitting.
> Do not publish my answers or announce my participation.

### Install (trusted checkout)

The `cargo install` below can replace a previously installed `discovery` CLI (for example, from an earlier survey); save that binary first if you still need it.

```bash
git clone https://github.com/MultiAgency/discovery
cd discovery
git checkout {{commit}}
cargo install --locked --path crates/cli
```

The CLI compiles in the locked release from this commit's `releases.json`. No
announcement or option can change it. The respondent skill is
`skills/discovery-respondent/SKILL.md` in the same checkout
([view](https://github.com/MultiAgency/discovery/blob/{{commit}}/skills/discovery-respondent/SKILL.md)).

### Verify

```bash
export DISCOVERY_PAYMENT_KEY={{payment_key}}
discovery verify {{survey_id}} --init-tx {{init_tx}} --target {{target}}
```

Continue only if it succeeds, its `release` is `{{release}}`,
`accepting_responses` is `true` and the target is `{{target}}`.
Verification checks on-chain that the survey was created by the registered
organizer, running the locked release, and that answers will be encrypted to a
key only that release can use.

```discovery-survey
release: {{release}}
survey: {{survey_id}}
init_tx: {{init_tx}}
deadline: {{deadline}}
payment_key: {{payment_key}}
```

## The questions

These are the survey's questions as `discovery verify` prints them. Each
answer names its question id.

{{questions}}

## What you approve

For each answer you choose:

- **Basis:** confirmed by you, observed by your agent, or your agent's
  interpretation. Report basis labels describe the cited answers; they do not
  mean you approved the model's generated finding.
- **Publication:** `publishable` ({{publishable}}) or `context_only`.
  Context-only answers shape the report but are never stated, unless you
  approve a specific phrasing.
{{withheld_terms}}
Submitted answers cannot be edited or withdrawn. Submitting the identical answers
again is safe and counts once. Full terms:
[service policy](https://github.com/MultiAgency/discovery/blob/{{commit}}/docs/policy.md).

## Submit

After you approve the exact `answers.json`, your agent runs:

```bash
discovery submit {{survey_id}} --init-tx {{init_tx}} --target {{target}} answers.json
```

It verifies the survey again, encrypts the answers on your machine and prints
"Submitted." on success. After a timeout or temporary service failure, wait
and retry with the same answers: they count once, even if the earlier attempt
was stored. Racing or interrupted attempts can still use additional places.
"survey is full" (at most 18 registrations) or "survey is closed" means this
attempt was refused. For a payment or access error, use the private help route
before retrying.

## Privacy limits

Only the synthesized report is published, or a withholding notice if no draft
passes the disclosure checks. Individual responses are not returned; authorized
text may appear in the report. Confidentiality is not anonymity: distinctive
details you approve could identify you to someone
who knows you. The disclosure checks are limited heuristics. OutLayer's public
job log shows submission counts and times. Please don't comment, react or
otherwise say publicly that you took part.

The payment key above is public and pays for submissions. If a submission fails
with a payment error, use the help route.

## Timing and results

The deadline is {{deadline}}. It is the earliest time we may finalize;
responses are accepted until finalization starts. The verified report will be
posted on [{{target}}]({{issue_url}}), followed by links to any issues created
or updated from it. Its footer gives a `discovery report …` command; run it to
check the posted report against the chain yourself. Verification authenticates
the released report; judge separately whether its findings reflect your
authorized needs. Proposed criteria require the maintainer's review against the
actual issue before any change.

## Private help

If you can't complete a submission, email [support@multiagency.ai](mailto:support@multiagency.ai). Send only what
you are comfortable sharing, without answers or identifying logs. Help requests
are not survey responses and will not be published or used as evidence unless
you separately agree.
