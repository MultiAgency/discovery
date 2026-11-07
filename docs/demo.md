# Demos

Two demos of one workflow on
[MultiAgency/discovery#1](https://github.com/MultiAgency/discovery/issues/1):
what stops people and their agents from joining a survey? The local demo
rehearses the protocol with invented answers. The live demo is the MVP
acceptance test.

## What each kind of evidence proves

| Evidence | Proves | Does not prove |
|---|---|---|
| Local tests and `scripts/demo.sh` | The survey, envelope, admission, publication-check, finalization and cleanup logic; CLI preflight, provenance parsing and invitation rendering, all against doubles | Deployed confidentiality, storage or keystore behavior, model quality, chain provenance |
| Deployed synthetic smoke test on `4adb3f91` ([evidence](release-evidence.md)) | The locked release on testnet: CLI bootstrap and survey verification, E2EE v3 inference, the full 18 registrations, release and refinalize, cleanup, an undelivered result recovered by retrying | That a person who isn't on the team can install, verify, answer and submit from the invitation alone; usefulness of findings from real answers |
| Not yet tested | An independent participant on a clean machine; a real survey's report; follow-up issues accepted by the maintainer | |

Known limit carried into the live demo: registrations contending with closing
can take one finalizer to 111 storage calls, above the intended 100-call
budget. A targeted regression test now reproduces that combined case and
successful cleanup ([release evidence](release-evidence.md)). Retry an
interrupted finalization after reconciling its transaction.

## Local walkthrough

```bash
scripts/demo.sh             # fails unless every step behaves as described
scripts/demo.sh demo-out    # also saves transcript.txt, released.md, withheld.md
```

The output directory must be new or empty; nothing is overwritten. It runs the
real survey, envelope, admission, publication-check, finalization and cleanup
code on the invented answers in [fixtures](../fixtures),
where context-only answer text is marked `SYNTHETIC`. Owner approval is encoded in the
fixtures, and storage, block time and the model are simulated; there are no
transactions. The transcript and both reports say so at the top. It does not
cover installation or CLI verification of the pinned release.

Presenter script, one line per step of the transcript:

1. **The request.** A survey on issue #1 with three public questions, its
   restrictions (quotes of publishable answers allowed), the deadline and the
   18-registration limit. The whole definition is public; its hash is the
   survey id.
2. **Approved contributions.** Each response's permissions without its text:
   per answer, its basis (confirmed, observed, interpretation) and how it may
   surface. Answers of different lengths seal to equal-size envelopes (the
   processing work still reveals length; see the [policy](policy.md#disclosure)).
   A retry with identical answers gets fresh ciphertext and counts once.
3. **Deadline semantics.** The deadline is the earliest finalization, not the
   end of collection: finalizing early is refused before any model call, and a
   response after the deadline is still accepted.
4. **Rejected draft, permitted report, cleanup.** The transcript shows, marked
   PRIVATE, a first draft naming a host from a context-only answer and the
   check results that only the model sees. The second draft is released with
   bases derived from the cited answers. Afterwards the survey is closed, its
   key and responses replaced by markers, submissions refused, and a
   finalization retry returns the same report without inference.
5. **Withholding.** A survey of context-only answers without phrasings: every
   draft fails the checks and only a fixed notice is published. A withheld
   report is a successful privacy outcome.

`released.md` and `withheld.md` use the public report renderer, with a
simulation banner and without the CLI's verified-transaction footer. The demo
asserts that neither contains fixture text, a context-only identifier or a
private source label. The drafts are scripted, so model quality and real
needs are evidence only from a live run.

## Live demo: invitation to actionable issues

The acceptance test: send one invitation link; anyone uses their existing agent
to try Discovery without live coaching, approves and submits feedback privately,
then sees the permitted report and useful follow-up issues. Success means the
feedback helps improve this product.

| Check | Evidence |
|---|---|
| Invitation stands alone | A participant follows the link without extra setup values or live coaching |
| Verified participation works | Post-lock survey verification and an accepted, owner-approved submission |
| Feedback reflects actual experience | Questions distinguish observed obstacles, assumptions and desired outcomes |
| Publication preserves permissions | Verified report or withholding notice; manual review before posting |
| Findings become actionable | Maintainer-approved new issue or update linked to a supporting public finding |
| A change helps | Implement one supported improvement and invite a participant to retry |

If a participant cannot reach submission, that is a product finding, not a
reason to coach them through silently. Request only minimal, sanitized details
through the private help route, and don't present help messages as survey
evidence unless their author separately approves publication.

### Runbook

Creation and finalization sign paid transactions as the organizer; invitation
rendering and survey verification also make a paid status call with the
submission key. Steps marked **public** publish to GitHub. Review costs and
publication with the maintainer. Commands follow the
[organizer skill](../skills/discovery-organizer/SKILL.md).

1. **Trusted checkout.** On a clean checkout of a commit pushed to the canonical
   `https://github.com/MultiAgency/discovery` repository (the `origin` remote),
   fetch origin and confirm the commit is published. Its `releases.json` must
   register `4adb3f91`. Run the checks in
   the [README](../README.md#quick-start), then
   `cargo install --locked --path crates/cli`.
2. **Definition.** Copy
   [fixtures/definition.json](../fixtures/definition.json)
   and set `deadline_ns` as text (jq would round it), then review it. Keep
   generated files outside the checkout:

   ```bash
   mkdir -p ../discovery-run
   deadline=$(( $(date +%s) + 5*86400 ))000000000
   sed "s/\"deadline_ns\": 0,/\"deadline_ns\": $deadline,/" fixtures/definition.json > ../discovery-run/definition.json
   discovery prepare init ../discovery-run/definition.json --target MultiAgency/discovery#1 > ../discovery-run/init-request.json
   ```

3. **Create (paid).** Sign `../discovery-run/init-request.json` with 0.1 NEAR as
   the organizer skill shows. Record the transaction hash and the output's
   `survey_id`.
4. **Invitation (paid status call).** Render it from a verification made now by
   this checkout's CLI, choosing where it will be published:

   ```bash
   export DISCOVERY_PAYMENT_KEY=<published submission key>
   scripts/invitation.sh <survey_id> <init_tx> MultiAgency/discovery#1 \
     "$(git rev-parse HEAD)" <https URL of the published page> > ../discovery-run/invitation.md
   ```

   It requires a clean tracked `HEAD`, the canonical `origin` URL and a cached
   origin branch containing the commit. This local check does not refresh refs
   or prove current remote availability. It also refuses another target, a
   survey that is not accepting responses or any failed verification. Questions,
   deadline and restrictions come from the authenticated definition. Review the
   page.
5. **Rehearse (paid verification and submission).** Privately give the rendered
   file to a teammate on a clean machine, with only their agent, before the page
   URL is public. They follow the file to an accepted submission of their own
   real experience. Record where they needed help; this uses a registration.
6. **Invite (public).** Publish the page at the URL given in step 4 and send
   the link privately within the remaining 18-registration capacity. The
   rehearsal uses a place; racing retries can consume additional places.
7. **Finalize (paid)** after the deadline: `discovery prepare finalize`, sign
   with 0.2 NEAR, then `discovery report … --target MultiAgency/discovery#1 >
   ../discovery-run/report.md`. On an `{"error": …}` output or a null result,
   reconcile the original transaction before another paid attempt. Once
   collection closes, retries resume finalization or cleanup; if a report was
   stored, they return that same report.
8. **Publish (public).** Post `../discovery-run/report.md` unchanged on issue #1.
9. **Follow up (public).** Propose issue updates or new issues from the posted
   findings as the organizer skill describes; create nothing without the
   maintainer's approval, then link them from issue #1. A withheld report
   supplies nothing to act on.
