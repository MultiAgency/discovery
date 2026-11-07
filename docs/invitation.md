# Try Discovery with your agent

<!-- TEMPLATE. Not an invitation until every <…> is filled from a verified
post-lock survey and this notice is removed. Before sharing: releases.json
registers the locked release, the synthetic smoke test passed including
full-size finalization, the questions below match the definition that
`discovery verify` prints word for word, and a teammate followed this page from
a clean machine without extra help. -->

We're testing Discovery, a confidential survey service for agents, by asking you
to try it. Your existing agent installs the CLI, verifies the survey, helps you
answer, shows you exactly what it will submit, and submits only what you
approve. Afterwards a synthesized report is posted on
[issue #1](https://github.com/MultiAgency/discovery/issues/1), and supported
findings become proposed GitHub issues.

We want to learn what joining is actually like, including what went wrong. If
you get stuck, that is a finding: tell us through the private help route below,
not in public.

## Before you start

- A machine where you can install software, with `git` and a Rust toolchain
  (`rustup`; tested with Rust 1.97). No wallet, NEAR account or GitHub account is
  needed to respond.
- An agent that can run shell commands and read files, such as Claude Code or
  Codex.
- About 15 minutes.

## Give this to your agent

> Help me respond to the Discovery survey described at <link to this page>.
> Install the CLI from the trusted checkout below and use the respondent skill
> from that checkout. Stop if the survey does not verify or the release does not
> match. Use only context I authorize and ask me about missing details. Show me
> every answer, its basis and its publication permission before submitting.
> Do not publish my answers or announce my participation.

### Install (trusted checkout)

```bash
git clone https://github.com/MultiAgency/discovery
cd discovery
git checkout <release commit>
cargo install --locked --path crates/cli
```

The CLI compiles in the locked release from this commit's `releases.json`. No
announcement or option can change it. The respondent skill is
`skills/discovery-respondent/SKILL.md` in the same checkout.

### Verify

```bash
export DISCOVERY_PAYMENT_KEY=<payment key>
discovery verify <survey id> --init-tx <creation tx> --target MultiAgency/discovery#1
```

Continue only if it succeeds, its `release` is `<release sha256>`,
`accepting_responses` is `true` and the target is `MultiAgency/discovery#1`.
Verification checks on-chain that the survey was created by the registered
organizer, running the locked release, and that answers will be encrypted to a
key only that release can use.

```discovery-survey
release: <release sha256>
survey: <survey id>
init_tx: <creation tx>
deadline: <UTC date and time; earliest finalization>
payment_key: <payment key>
```

## The questions

1. What were you or your agent trying to accomplish when preparing to join a
   survey? Describe actual experience, or say you have not tried it.
2. What was difficult, unclear or error-prone? What did you try, and what
   happened? Distinguish observations from assumptions.
3. What would joining need to make possible, and what verification or disclosure
   control must it preserve?

Trying this survey counts as experience for question 1.

## What you approve

For each answer you choose:

- **Basis:** confirmed by you, observed by your agent, or your agent's
  interpretation.
- **Publication:** `publishable` (may appear paraphrased or in short quotes,
  so write it as you'd be comfortable seeing it in public) or `context_only`.
  Context-only answers shape the report but are never stated, unless you
  approve a specific phrasing.

Submitted answers cannot be edited or withdrawn. Submitting the identical answers
again is safe and counts once. Full terms:
[service policy](https://github.com/MultiAgency/discovery/blob/main/docs/policy.md).

## Submit

After you approve the exact `answers.json`, your agent runs:

```bash
discovery submit <survey id> --init-tx <creation tx> --target MultiAgency/discovery#1 answers.json
```

It verifies the survey again, encrypts the answers on your machine and prints
"Submitted." on success. If it fails or times out, retry with the same answers:
they count once, even if the earlier attempt was stored. "survey is full" (at
most 18 responses) or "survey is closed" means this attempt was refused.

## Privacy limits

Only the synthesized report is published, or a withholding notice if no draft
passes the disclosure checks. No individual answer is published. Confidentiality
is not anonymity: distinctive details you approve could identify you to someone
who knows you. OutLayer's public job log shows submission counts and times.
Please don't comment, react or otherwise say publicly that you took part.

The payment key above is public and pays for submissions. If a submission fails
with a payment error, use the help route.

## Timing and results

The deadline is <UTC date and time>. It is the earliest time we may finalize;
responses are accepted until finalization starts. The verified report will be
posted on [issue #1](https://github.com/MultiAgency/discovery/issues/1), followed
by links to any issues created or updated from it. Its footer gives a
`discovery report …` command; run it to check the posted report against the
chain yourself.

## Private help

If you can't complete a submission, email [support@multiagency.ai](mailto:support@multiagency.ai). Send only what
you are comfortable sharing, without answers or identifying logs. Help requests
are not survey responses and will not be published or used as evidence unless
you separately agree.
