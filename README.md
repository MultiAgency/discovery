# Confidential surveys for agents

A maintainer asks for context about an existing GitHub issue. People's personal
agents submit authorized, encrypted responses. After collection closes,
confidential processing releases a report under the disclosure rules, or a
withholding notice, and the maintainer posts it to the issue.

**Existing issue → survey → private responses → permitted report → better requirements.**

Existing agents provide the interface; respondents need no wallet or identity
key. [The concept](docs/concept.md) defines the product. This README is the
service policy and operating guide. Role guidance lives in the
[organizer](skills/discovery-organizer/SKILL.md) and
[respondent](skills/discovery-respondent/SKILL.md) skills.

## Readiness

**Live use is not ready.**

- [releases.json](releases.json) registers no service, so the CLI refuses live use.
- An unlocked deployment of the current build completed one synthetic
  request-to-report cycle ([deployed synthetic test](#deployed-synthetic-test-2026-11-06)).
  The owner is not locked, so `verify` and `report` refuse it, and the
  [synthetic smoke test](#synthetic-smoke-test) against a locked release has not run.

## Quick start

Tested with Rust 1.97.0. The service build needs the `wasm32-wasip2` target.
These checks need no credentials and also run in CI:

```bash
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --locked --target wasm32-wasip2 -p discovery-module
```

### Local demo

One command, no credentials. It runs the real survey, envelope, admission,
publication-check, finalization and cleanup code on the invented answers in
[scripts/synthetic](scripts/synthetic). Storage, block time and the model are
simulated and there are no transactions, so it shows the protocol but says
nothing about deployed privacy. Nothing it prints is chain-verified.

```bash
scripts/demo.sh            # artifacts in target/demo/
```

It exits non-zero unless every outcome in `outcomes.json` passes:

| # | Outcome |
|---|---|
| 1 | a survey is created for `MultiAgency/discovery#1` |
| 2 | distinct responses with publishable and context-only answers, all three bases, are accepted |
| 3 | an identical retry is re-encrypted (new ciphertext) and stored once |
| 4 | finalizing before the deadline fails without any model call |
| 5 | after the deadline, collection stays open and a late response is accepted |
| 6 | the released report has findings with derived bases and acceptance-criterion changes |
| 7a | a draft naming a context-only host is rejected and a corrected draft released |
| 7b | a separate survey whose every attempt leaks is withheld |
| 8 | the survey is closed, its key and responses replaced by markers, submissions refused, and a finalization retry returns the identical report without inference |

`report.md` and `withheld.md` are the public reports. The demo fails if either
contains fixture text, a context-only identifier, a private source label, a
respondent count or rejection details. `transcript.txt` and `outcomes.json` are
held to the same checks except the last: they say that a draft was rejected,
and name the reason only by category.

For real surveys, install the CLI from a trusted checkout and run it from any
directory. Give your agent the issue URL and the matching skill from that checkout:

```bash
cargo install --locked --path crates/cli
discovery --help
```

The CLI compiles in the service from this checkout's [releases.json](releases.json).
Its project, build hash and organizer cannot be overridden by an announcement or
option. The only setting is `DISCOVERY_PAYMENT_KEY`, the published submission
key. Testnet RPC, API and contract endpoints are fixed.

| Command | Who | Does |
|---|---|---|
| `prepare init <definition.json>` | organizer | validate; print unsigned creation arguments to sign with `near` |
| `verify <survey> --init-tx <tx>` | both | verify the survey on-chain against the release; show definition and state |
| `submit <survey> --init-tx <tx> <answers.json>` | respondent | verify, encrypt to the verified key, submit |
| `prepare finalize <survey>` | organizer | print unsigned finalization arguments to sign with `near` |
| `report <survey> --finalize-tx <tx>` | both | verify the report on-chain and print Markdown |

`--target OWNER/REPO#NUMBER` makes `verify`, `submit`, `report` and
`prepare init` refuse any other issue.

Source layout: `crates/protocol` holds the shared formats and response
encryption; `crates/module` is the OutLayer service (storage, collection,
NEAR AI inference, publication checks, cleanup); `crates/cli` is the client.

## Service policy

### Authorization

The maintainer approves the issue, questions, deadline and restrictions. A
personal agent gets its human's approval of the final answers, their basis and
their disclosure tiers before submitting. Skills guide that approval; the service
cannot prove a human gave it. The full survey definition is public, including
withheld terms, even if the issue is private.

### Disclosure

Only the report is released; no operation returns individual responses.
Responses are encrypted to a key that only the locked, reviewed release can use.
Respondents verify that key on-chain before encrypting to it.

- Each answer is `publishable` or `context_only`. A context-only fact can
  surface only through the respondent's approved phrasing; without one it may
  shape the synthesis but must not be stated.
- Organizer restrictions (`no_direct_quotes`, `withheld_terms`) can only tighten
  publication.
- Each finding must cite authorized answers. Its basis (confirmed by a human,
  observed by an agent, or interpretation) comes from those answers, not from
  the model. Citations stay private. A citation proves permission to draw on an
  answer, not that the answer supports the statement; the summary, disagreements,
  open questions and acceptance criteria cite nothing. Citations the model puts
  on proposed acceptance-criteria changes are accepted and discarded; only
  findings carry a basis.
- Mechanical checks block runs of six or more confidential words, recognized
  identifier-shaped tokens and withheld terms. Survey wording and approved
  material are exempt. The checks cannot recognize every personal detail or
  semantic disclosure. A separate model audit is best-effort.
- A rejected draft is regenerated, up to three attempts in all. If every attempt
  fails, only a fixed withholding notice is released. Rejection reasons
  (including format errors, which go back to the model inside the enclave) and
  provider errors are never published.

There is no anonymity promise. Distinctive approved details can identify someone.
OutLayer's public job log shows submission counts and times under the shared
service account. Anyone with the payment key can contribute, so counts are
context, not proof of usage or a vote. Respondents must not announce their
participation.

### Collection and retention

Finalization closes admission and freezes the collection; an in-flight
submission is included only if its write wins.
Responses are immutable, with no editing or withdrawal. Identical answer
batches contribute once, including re-encrypted retries and identical batches
from different agents.

Each response is at most 4,092 bytes of serialized JSON. A survey admits
256 KiB of canonical response JSON in total: 64 maximum-size responses when
no registration is duplicated or abandoned. Beyond that, submissions fail with "survey is full". There is no count cap or
sampling. Every response uses one 4 KiB padding size (including its length prefix).
Retries of admitted responses still succeed. Identical submissions that race,
or a retry after an interrupted submission, may be charged against the budget
again each time; they still contribute once. The 256 KiB budget is a judgment value to
check by timing the first deployed finalization: at full budget each model
request carries roughly 260–310 KB of prompt, and finalization makes up to six
(three drafts, three audits) in one 180-second job.

Flooding with the public payment key can fill the survey early instead of
admitting an unbounded batch that blocks finalization. Recover by creating a new
survey; abuse handling remains open and there is no Sybil resistance.

Encrypted responses are kept until a finalization succeeds; they have no
independent expiry. Once the report is stored, response contents and the survey
key are replaced with sealed empty markers. Retries resume this cleanup without
producing a second report. Registration metadata and markers remain to fence
delayed writes. Storage backups and physical erasure are not verified.

One report is stored per survey. Operator retries and competing finalizers may
run inference more than once. Model attestation evidence is not kept.

### Trust

The service relies on OutLayer's worker and keystore governance, NEAR AI's
attested model, and the NEAR RPC used for verification. The CLI checks that the
creation and finalization transactions came from the organizer, ran the
registered project and build, and that at that block the build was the project's
only version and its owner was an initialized account with no access keys or
contract code. Before sending any response content, the module checks the
gateway's and model's Intel TDX quotes, NVIDIA's GPU verdict and the model's
attested signing key, all bound to a fresh nonce. It encrypts each request to
that key and uses a reply only after the model's signature over the exact
request and reply checks out.

The MVP trusts NEAR AI to manage which software runs on that attested hardware.
NEAR AI deploys model containers frequently, and applications choose their own
[verification policy](https://docs.near.ai/cloud/verification/reference/verification-policy).
Hardware attestation does not establish that independently reviewed model code
processed a response; this service does not check that. Our own code is
independently pinned: the locked OutLayer release enforces contributor
permissions, publication checks and recoverable cleanup.

### MVP versus concept

- The organizer finalizes after the deadline; nothing is scheduled. Collection
  remains open until finalization starts.
- The organizer posts the verified report with `gh`.
- A GitHub issue is required, but the service and Discovery CLI make no GitHub calls.
- Responses stay encrypted until finalization succeeds, with no independent expiry.
- Confidentiality does not guarantee anonymity; approved details may identify someone.

## Release setup

Existing projects are immutable; new code and formats ship as a new locked
project. Use the official [OutLayer CLI skill](https://skills.outlayer.ai/outlayer-cli/SKILL.md)
for deployment, secrets and payment keys. `discovery` refuses the release
until it is locked, and a survey created before the lock never verifies.

1. Freeze the source, then build and review the module and deploy those exact
   bytes to a new project. Build outside the shared `target/`, hash the copy you
   upload, and keep the source it came from, including untracked files:

   ```bash
   dir=~/.discovery-release; mkdir -p "$dir"
   CARGO_TARGET_DIR="$dir/target" cargo build --release --locked --target wasm32-wasip2 -p discovery-module
   cp "$dir/target/wasm32-wasip2/release/discovery-module.wasm" "$dir/release.wasm"
   shasum -a 256 "$dir/release.wasm"     # the release hash; upload this file
   git ls-files -co --exclude-standard -z | tar --null -T - -czf "$dir/source.tgz"
   ```
2. Use a fresh, initialized NEAR account as the release owner and never deploy
   code to it. Keep only the reviewed version, then delete all of the owner's
   access keys.
3. Use a separate funded organizer account. Give it a `nearai` secrets profile
   holding `NEARAI_API_KEY`, restricted to the reviewed build. Create the
   shared submission payment key under the same account, since the caller selects
   the storage namespace: the key must start with `<organizer>:`. `outlayer`
   0.2.1 cannot restrict a key to a project or cap it per call, so it is not
   restricted to this project, and anyone holding it can spend its whole
   balance. Keep the balance small and
   monitor it. It is published; the organizer's NEAR key is not. Also set a
   [spend limit](https://docs.near.ai/api-reference/workspaces/update-api-key-spend-limit.md)
   on the NEAR AI API key itself.
4. Set the service in `releases.json`:
   `{"service": {"project": "<owner/project>", "release": "<wasm sha256>", "organizer": "<account>"}}`,
   then reinstall the CLI. The CLI refuses a malformed entry or an organizer that
   is the project owner. Finish open surveys before replacing it.
5. Pass the [synthetic smoke test](#synthetic-smoke-test) before collecting
   personal data.

Install the official NEAR CLI. Its legacy keychain
(`~/.near-credentials/testnet/<organizer>.json`) holds the organizer key;
`discovery` never handles it:

```bash
cargo install near-cli-rs --version 0.30.1 --locked --no-default-features
```

## Operating a survey

Use one explicit `OWNER/REPO#NUMBER` throughout; the working directory does not
determine the target. GitHub access needs only reading the issue and commenting.
Creation and finalization are on-chain calls, because only those carry trusted
block time.

### Create and announce

```bash
gh api repos/OWNER/REPO/issues/NUMBER        # confirm it is an issue, not a PR
discovery prepare init definition.json --target OWNER/REPO#NUMBER > init-request.json
```

`definition.json` follows the organizer skill's format. Sign the request with the
service's organizer account. The deposit (0.2 NEAR for finalization) and limits
are testnet starting values, so review the cost before signing:

```bash
near --quiet contract call-function as-transaction outlayer.testnet request_execution \
  file-args init-request.json prepaid-gas '300 Tgas' attached-deposit '0.1 NEAR' \
  sign-as <organizer> network-config testnet sign-with-legacy-keychain send
```

 A successful transaction can still carry a module `error`. Record the
transaction hash and the output's `survey_id`, then verify before announcing:

```bash
discovery verify <survey> --init-tx <init_tx> --target OWNER/REPO#NUMBER
gh issue comment NUMBER --repo OWNER/REPO --body-file announcement.md
```

### Finalize and publish

After the deadline, prepare finalization and sign `finalize-request.json` the same
way, then fetch and post the verified report:

```bash
discovery prepare finalize <survey> > finalize-request.json
discovery report <survey> --finalize-tx <finalize_tx> --target OWNER/REPO#NUMBER > report.md
gh issue comment NUMBER --repo OWNER/REPO --body-file report.md
```

The report's footer names its transaction and release, so anyone can re-verify
it. A withheld report is a successful privacy outcome. Interpret findings with
the maintainer and get authorization before editing the issue.

### Completion and failures

A finalization transaction whose module output is a report has finished cleanup,
including when the report is withheld. An `{"error": ...}` output means
finalization did not complete: reconcile the transaction, then retry finalization.
Retries resume cleanup and return the same stored report.

Optionally double-check with `discovery verify`: it should show `closed: true`
and `key_present: false`, and further submissions must fail. Never delete the
sealed markers or registration metadata.

After a timeout, reconcile the original transaction before signing another paid
attempt. Never blindly sign a replacement:

```bash
near transaction view-status <transaction_hash> network-config testnet
```

To keep a recovery artifact, sign with `save-to-file signed.json` instead of
`send`, then broadcast it with `near transaction send-signed-transaction`.

Use the same service and CLI build for a survey's lifetime.

## Synthetic smoke test

Run it once per release, on testnet, after the owner is locked, with the
invented answers in [scripts/synthetic](scripts/synthetic) only. Every
`create`, `early`, `finalize` and `refinalize` is a paid call signed by the
organizer. [scripts/smoke.sh](scripts/smoke.sh) saves each signed transaction
and records its hash in `~/.discovery-smoke/<name>/transactions.log` before
broadcasting. Re-running a step whose outcome is unknown re-broadcasts that same
transaction rather than signing a new one; `smoke.sh status <name> <label>`
reconciles it. Record OutLayer job durations alongside.

```bash
export DISCOVERY_PAYMENT_KEY=<organizer:nonce:secret>
s=scripts/smoke.sh; a=scripts/synthetic
$s create main 10                          # 1: create, verify, print announcement fields
$s early main                              # 1: refused before the deadline
$s submit main $a/answers1.json            # 2
$s submit main $a/answers2.json
$s submit main $a/answers1.json            # 2: identical retry
$s submit main $a/answers3.json            # 3: after the deadline, still accepted
$s finalize main                           # 3–4: verified report.md, public checks
$s check main                              # 5: closed, key gone, submissions refused
$s refinalize main                         # 5: identical output, short job
$s create quiet 5                          # 6: context-only, no phrasing
$s submit quiet $a/context-only.json
$s finalize quiet                          #    after its deadline
$s workload 64 ~/.discovery-smoke/workload # 7: about the full 256 KiB budget
$s create load 30
for f in ~/.discovery-smoke/workload/workload-*.json; do $s submit load "$f"; done
$s finalize load                           #    after its deadline; time the job
```

1. **Create and verify.** `create` writes a fresh definition (deadline in
   minutes, `no_direct_quotes: true`), records the creation transaction, checks
   the module output and runs `discovery verify`, which must show
   `accepting_responses: true`. `early` must return "the collection period has
   not ended".
2. **Submit and retry.** Three distinct responses mix both tiers and all three
   bases, with context-only identifiers. One is submitted a second time.
3. **Finalize after the deadline.** Submit once more after the deadline (still
   accepted), then finalize. A `{"error": ...}` output stops the script; run
   `finalize` again to retry. A fully final on-chain transaction failure is
   recorded separately in `<label>.failure.json`; fix its cause and rerun the
   step for a fresh transaction. Unknown or non-final outcomes retain the
   original signed transaction for reconciliation and rebroadcast.
4. **Retrieve and verify.** `discovery report` must verify the transaction and
   release. The script rejects fixture text, identifiers, source labels,
   respondent counts and rejection details. Check by hand that at least one
   acceptance-criterion change or corrected assumption is grounded in the
   answers.
5. **Check completion.** `check` requires `closed: true`, `key_present: false`
   and "survey is closed" for a new submission. `refinalize` must return the
   identical output without inference.
6. **Context-only handling.** A second survey with only `context_only` answers
   without phrasing. Either a withholding notice or a report that passes manual
   review for leaks is acceptable; anything else blocks release. A clean report
   shows safe handling of that sample, not deployed withholding, which only
   local tests establish.
7. **Time the admitted workload.** Finalize a small batch first, then larger
   ones toward the budget: `workload 64` writes distinct ~4 KB responses whose
   total is about 256 KiB, so the last may be refused as "survey is full". If
   the full budget lacks runtime headroom in the 180-second job, or the model
   refuses the input, lower `BUDGET` in a new release. Collect no personal data
   until the full budget fits.

Local tests already establish encryption binding, deduplication, admission and
freeze fences, publication checks, withholding, single-report storage, cleanup
markers and confidential-safe errors, all against storage and model doubles.
A deployed run adds evidence for the storage behavior it exercises (not CAS
semantics in general), keystore and attestation behavior, model output quality
and limits, job timing, cost, and on-chain provenance as the CLI sees it.

### Deployed synthetic test (2026-11-06)

Project `research.agency.testnet/discovery`, one version
`a73c24e52229e761238fba0dc58db5eb9ac4d5f823b59ff44729d35bf9ead97d`, organizer
`surveys.research.agency.testnet`. The owner keeps its key: this run is
unlocked, so the CLI's `verify` and `report` correctly refuse it. Creation and
finalization used `near`, and submissions used `outlayer run`.

- Survey `4424df37…`: two synthetic responses plus an identical retry, with
  planted emails, paths, hosts, a NEAR account, an IP, a ticket number and a
  withheld term, under `no_direct_quotes`. Finalization (tx `3tBUZJSP…`)
  released 4 findings with derived bases, 2 acceptance-criteria changes and 2
  open questions, with none of the planted material, no six-word quotes and no
  source labels. Afterwards the survey was closed, its key erased, and a late
  submission rejected.
- Timing: creation 2–3 s of execution (about 24 s end to end); submissions
  2–3 s; finalization 58 s of execution (76 s end to end), against the 180 s
  limit. The full admission budget is untimed.
- Cost: creation about 0.0013 NEAR and finalization about 0.0068 NEAR; the rest
  of each 0.1/0.2 NEAR deposit was refunded. HTTPS calls cost about $0.001 each.
  The whole run, including setup, both surveys, a discarded first upload and
  the version swap, spent about 0.06 NEAR: 0.027 from the owner and 0.033 from
  the organizer.
- The first attempt (survey `93398d9b…`, build `554cc8bb…`) was withheld with no
  findings. The model's drafts failed the format and wording checks: citations
  on criteria, open questions as objects, Markdown code fences, and verbatim
  reuse of publishable wording under `no_direct_quotes`. The fixes state exact
  field shapes, name the failed field on retry, strip one code fence, discard
  criterion citations, label strict-mode publishable text `paraphrase_only`,
  and allow three attempts. In a native rerun against the live model, 6 of 6
  runs released.

## Own-repository demo

Research onboarding on [Discovery issue #1](https://github.com/MultiAgency/discovery/issues/1):
what stops people and their agents from joining a survey? After release setup,
create a survey with a fresh deadline, `no_direct_quotes: true`, and:

1. What were you or your agent trying to accomplish when preparing to join a
   survey? Describe actual experience, or say you have not tried it.
2. What was difficult, unclear or error-prone? What did you try, and what
   happened? Distinguish observations from assumptions.
3. What would joining need to make possible, and what verification or disclosure
   control must it preserve?

Invite teammates through their own agents:

> Help me respond to the survey at <announcement link>. Use the respondent skill
> from our trusted Discovery checkout. Use only authorized context and ask me
> about missing details. Show me the answers, their basis and publication
> permissions before submitting. Do not publish my answers or announce my
> participation. Stop if the survey does not verify.

After the deadline, finalize, post the report and check completion. Success is
an acceptance criterion changed or an assumption corrected on the issue, with
identifying material withheld.
