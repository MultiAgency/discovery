# Demos

Two demos of one workflow on
[MultiAgency/discovery#1](https://github.com/MultiAgency/discovery/issues/1):
what stops people and their agents from joining a survey? The local demo
rehearses the protocol with invented answers. The live demo is the MVP
acceptance test, which waits on the [readiness](../README.md#readiness) gates.

## Local walkthrough

```bash
scripts/demo.sh        # fails unless every step behaves as described
```

It runs the real survey, envelope, admission, publication-check, finalization
and cleanup code on the invented answers in
[scripts/synthetic](../scripts/synthetic), where every answer text is marked
`SYNTHETIC`. Storage, block time and the model are simulated, owner approval is
encoded in the fixtures rather than obtained, and there are no transactions. It
shows the protocol, not deployed confidentiality, and does not cover
installation or CLI verification of the pinned release. It prints five steps:

1. **The request.** A survey on issue #1 with three public questions. It
   allows short quotes of publishable answers; forbidding them
   (`no_direct_quotes`) makes withholding much more likely. The whole
   definition is public.
2. **Approved contributions.** Each response's permissions, without its text:
   per answer, its basis (confirmed, observed, interpretation) and how it may
   surface. The options are publishable and paraphrased, context-only through an approved phrasing,
   or context-only and never stated. Responses are encrypted to the survey key;
   every envelope is padded to the same size, so its length doesn't reveal the
   answer's length. A retry with identical answers gets fresh ciphertext and
   counts once.
3. **Collection closure.** The deadline is the earliest finalization time, not
   the end of collection. Finalizing early is refused before any model call,
   and a response after the deadline is still accepted.
4. **Permitted findings.** The first draft names a private host from a
   context-only answer. The mechanical checks reject it, the reason goes back to
   the model only, and the second draft is released. Findings carry bases
   derived from the answers they cite, and citations stay private. Afterwards
   the survey is closed, its key and responses replaced by markers, submissions
   refused, and a finalization retry returns the same report without inference.
5. **Withholding.** A separate survey of context-only answers without
   phrasings, where every draft fails the checks: only a fixed notice is
   published. A withheld report is a successful privacy outcome.

The released report proposes two acceptance-criterion changes: report every
missing local setting before contacting any service, and state that responses
are accepted until finalization starts. The demo asserts that neither public
report contains fixture text, a context-only identifier or a private source
label. The drafts are scripted and come from invented answers, so model quality,
the audit and real user needs are evidence only from a live run.

## Live demo: invitation to actionable issues

The acceptance test: send one invitation link; anyone uses their existing agent
to try Discovery without live coaching, approves and submits feedback privately,
then sees the permitted report and useful follow-up issues. Success means the
feedback helps improve this product.

The entry point is [invitation.md](invitation.md), a template until it is filled
from a verified survey. It must stand alone: the trusted checkout and install
commands, the respondent skill, the survey announcement, what verification
establishes and what the owner approves, the deadline, where results appear, and
a private help route. The survey uses the questions in
[scripts/synthetic/definition.json](../scripts/synthetic/definition.json) with a
fresh deadline; the invitation's questions must match the definition that
`discovery verify` prints word for word.

If a participant cannot reach submission, that is a product finding, not a
reason to coach them through silently. Request only minimal, sanitized details
through the private help route, and don't present help messages as survey
evidence unless their author separately approves publication.

After the deadline the organizer finalizes, posts the verified report unchanged
and proposes follow-up issues from it, as the
[organizer skill](../skills/discovery-organizer/SKILL.md) describes.

| Check | Evidence |
|---|---|
| Invitation stands alone | A participant follows the link without extra setup values or live coaching |
| Verified participation works | Post-lock survey verification and an accepted, owner-approved submission |
| Feedback reflects actual experience | Questions distinguish observed obstacles, assumptions and desired outcomes |
| Publication preserves permissions | Verified report or withholding notice; manual review before posting |
| Findings become actionable | Maintainer-approved new issue or update linked to a supporting public finding |
| A change helps | Implement one supported improvement and invite a participant to retry |

Before opening participation: lock and register the release, pass the
[synthetic smoke test](release.md#synthetic-smoke-test) against it, publish the
trusted installation path, prepare and verify a fresh survey, and fill and
rehearse the invitation. Organizer-triggered finalization and approved manual
publication are sufficient for this MVP; a frontend or scheduler is not
required.
