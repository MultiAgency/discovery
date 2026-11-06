> Historical material. The current scope is [the November 4 concept](../concept.md); these documents do not define current features or operating instructions.

# Decisions

Running log from the stress-test. Feeds the requirements pass.

## D1. Threat model: organizer and operator, plus re-identification from the report

- Respondents are protected from the survey organizer **and** from us, the service operator. Every link that touches plaintext must run in attested code. Operator trust is never used as a fallback; links that can't be attested in time are redesigned or cut, not disclosed.
- The published report must not let readers re-identify respondents. That is a property of the output, enforced by publication rules, not by the TEE.

## D2. Promise: private redaction, not anonymity in a crowd

- The service is a confidential intermediary. The organizer sees only what passes the publication rules. n=1 is allowed.
- Protection rests on (1) the publication rules applied to the output and (2) participation being hidden.
- Stated limitation: content that passes the rules can still be linked to a respondent if the organizer can guess who responded.
- A minimum-respondent floor (k) is deferred to an optional setting later. It needs defences against fake respondents first, because k−1 fake responses isolate the one real respondent.

## D3. Disclosure tiers

- Mechanical rule as implemented (`crates/module/src/checks.rs`): the report may never repeat 6+ consecutive words of `context_only` material (text, evidence, repro steps). It may repeat `publishable` text only if the organizer allows quotes. Approved phrasings and the survey's own wording are always allowed. Identifier-shaped tokens (emails, URLs, domains, NEAR accounts, paths, handles, long hex or numbers) from `context_only` material and withheld terms are blocked.
- Model audit as implemented: it gets the authorized material (publishable text, repro steps, approved phrasings), the confidential material and the report, and flags only what the authorized material doesn't support. An earlier version saw only the confidential facts and withheld reports for repeating what respondents had themselves published (diagnosed 2026-11-05).
- Retry feedback names the offending text. It goes only to the model, inside the enclave.
- **Rejection reasons are never published.** They would restate confidential facts. Combined with organizer-chosen `withheld_terms`, they'd also be a membership probe ("withheld: withheld term" means someone mentioned that company). A `diagnostics` build feature puts them in withheld reports, for the dev project on synthetic data only. Respondents pin the release hash, so a diagnostics build can't run a real survey.
- `publishable` items, plus `context-only` items made of a raw fact and a respondent-approved `as` phrasing, which is the only form that fact may take in the report. Unmarked items count as `context-only` with no phrasing. The mechanical check is the stated guarantee: no verbatim overlap with `context-only` text or with any other respondent's text, and no identifiers. The model audit is best-effort. On failure: regenerate once, then publish only "could not be released" and discard the responses. Disclosed limit: combinations of approved phrasings can still identify someone.

## D4. Feedback → product direction happens outside the TEE

- Finalization publishes one **structured report**: themes, affected workflows, disagreements, proposed changes to acceptance criteria, suggested task breakdowns, confidence levels. It is machine-readable JSON plus readable Markdown, and it is the only thing released.
- **Posting:** as the concept says (§4.5, "the service posts the permitted report on the issue"), the `finalize` Action posts the report automatically, with its provenance line (D12). The maintainer requested it by posting `/survey`, and the announcement promised respondents a report after the deadline. Maintainers can hide a posted comment like any other, and the report stays public on-chain either way. The GitHub credential lives in the Action, never in the enclave. Model text is escaped (no `@mentions`, no HTML).
- **Acting:** the maintainer's agent (organizer skill) turns the report into acceptance-criteria changes, sub-issues, tasks and board items with the maintainer's own token. The maintainer decides each one; a human approves.
- Why: one release, one filter, one privacy boundary. Tasks and boards can only contain what the report already disclosed. The enclave holds no GitHub access. No automatic prioritization.
- "Qualified feedback" means **well-formed**: confirmed answers kept separate from the agent's observations and interpretations. It does not mean verified usage, which stays out of scope.

## D5. One project per release, locked owner

- **Each release is its own OutLayer project,** owned by a fresh subaccount (e.g. `v1.research.agency.testnet`). Deploy the audited build, remove any other versions, then delete the subaccount's keys. Nobody can then add or switch versions or transfer the project, because those all require the owner (`contract/src/projects.rs`). Fixes ship as a new project; open surveys finish on the old one.
- **Keys are project-bound encryption keys** (`bind: "project"`, `caller: "signer"`), derived as `HMAC(master, "encryption-key:v1:project:{project_uuid}:signer:{account}:{path}")`. No `PROTECTED_*` secrets.
- **One service account for every call:** `research.agency.testnet` on testnet. The key and storage cell depend on the caller, and encryption keys can't use `caller: "none"` (refused in `keystore-worker/src/encryption_keys.rs:310`).
  - **`Submit`:** HTTPS with a payment key that the service account owns, restricted to this project, with a cap per call. It's published in the CLI. Misuse can only drain its balance.
  - **`InitSurvey` and `Finalize`:** NEAR transactions signed by the service account, which also gives D10 its trusted `NEAR_BLOCK_TIMESTAMP`. The module refuses `Finalize` over HTTPS.
  - The service account signs calls to OutLayer only. Any contract it interacts with could start a run under its key with chosen input. That's harmless because the module never outputs plaintext and `Finalize` runs once.
- **Rejected:**
  - *Build-bound keys:* only allowed on direct runs from a WASM URL (`worker/src/encryption_keys/mod.rs:13`). Those runs have no storage (`worker/src/main.rs:3152`) and can't be called over HTTPS.
  - *Owner-trust fallback:* violates D1. OutLayer's private-DAO example simply assumes the owner is honest.
- **What we can still do, disclosed:** decline to send `Finalize`, which affects availability only.
- **Disclosed trust root:** OutLayer's keystore and administrators (`outlayer.near`, `worker.outlayer.near`, `dao.outlayer.near`). Don't use MPC Vaults.
- **Testnet checks: all passed (2026-11-05, `spike/RESULTS.md`).** First locked release: `rel1.research.agency.testnet/discovery` @ `74bd94d7…c3fa`.
  1. A project with a keyless owner still runs over HTTPS and on-chain.
  2. `add_version` fails, and removed versions can't be run.
  3. The key and storage are the same for HTTPS calls (payment key owned by the service account) and on-chain calls (signed by it).
  4. `NEAR_BLOCK_TIMESTAMP` is present on-chain.
  5. `set-if-absent` protects the survey keypair.

## D6. Respondent identity: local key, with an optional custody-wallet anchor

- **Every submission is signed by a local key** in NEP-413 format. The signature and public key travel inside the ciphertext. The TEE verifies the signature against the included key, with no NEAR RPC lookup ("for implicit accounts: `account_id == hex(public_key_bytes)` — no RPC needed", agent-custody `references/wallet-ops.md`).
- **Unanchored (default):** the CLI derives a separate key for each survey from a local secret (HKDF). Identity is that key. The record is named `mac(survey_id ‖ key)`. A respondent can't be linked across surveys.
- **Anchored (custody wallet):** once, at setup, the agent's OutLayer custody wallet signs `anchor:<P>` via `/wallet/v1/sign-message`, where P is a stable local key. Submissions are signed by P and carry the anchor inside the ciphertext. The TEE verifies the chain wallet → P → response. Identity is the wallet. The record is named `mac(survey_id ‖ wallet)`.
- **Why the anchor is signed once:** with OutLayer as organizer (D8), signing each submission through the custody API would let OutLayer's front end link wallets to submissions, which breaks D1. A single signing event at setup isn't tied to any survey or time.
- **Disclosed:**
  - Anchoring tells OutLayer's front end that the wallet set up this service, but not which surveys it answered or when.
  - Inside the TEE, anchored responses can be linked across surveys.
  - Wallets and keys are free to create, so there's no flood resistance yet.
- **Floods:** the TEE merges near-identical responses before synthesis, and the report states the response and merge counts.
- **Later:** survey-set gates on anchored identity: deposit or payment check, balance, named account via binding, account age.
- **Testnet checks:**
  - custody `sign-message` output verifies inside the module;
  - custody signing doesn't appear in the public job log;
  - ed25519, HKDF and NEP-413 serialization fit in the WASM module.

## D7. Submission timing

- Accept and disclose: OutLayer's public job log shows the number and timing of submissions, never who made them.
- The respondent skill tells agents not to announce participation on the issue, so there's no public signal to line up with a job's timestamp.
- The public job log also shows `instructions_used` for each run, which varies with input size (spike, 2026-11-05). The CLI pads ciphertexts to fixed size buckets so it doesn't reveal response length.
- Rejected: a random submission delay. The attack is narrow, and a delay of minutes or hours forces the CLI to stay running or schedule a later submission. Decoy traffic can be revisited later.

## D8. Target: our own repo

- Surveys and product reviews run on `MultiAgency/discovery`'s own issues and PRs (private repo) throughout development. Respondents are the agents building this project, reporting their own working experience, plus humans where relevant.
- Agent respondents answer from their own working history. The report distinguishes "observed by an agent while working" from "confirmed by a human".
- Known limit: organizer and respondents are largely the same people, so the target exercises the full loop but doesn't demonstrate the privacy guarantee to an outside party. The external demo issue (`out-layer/outlayer#34` was the candidate) has been dropped for now.

## D9. Product review is part of the process

- Lifecycle: **issue survey → report (acceptance criteria and tasks, D4) → PR → product review → review report on the PR.**
- A product review is a survey aimed at a PR. Respondents' agents try the PR build against their real setups and report findings in code-review style: severity, the acceptance criterion concerned, and a failure scenario. The review report gives a verdict per criterion, e.g. "met for 4 of 5 setups; failed for 1 (multi-merchant flows)".
- Why it needs privacy: testing a fix against a real setup exposes that setup.
- It reuses the same machinery: the survey target is a PR instead of an issue, with a review template.
- Respondent skill rules: only run PR builds from maintainers or trusted contributors, or run them against a copy of the real setup. Get the owner's approval before running any unmerged code.
- Anyone with access can review a PR for now; on our own repo that's the agents building it. Later: limit reviews to the issue survey's original respondents via a signed receipt the respondent holds, so the enclave keeps nothing after finalization.

## D10. Timed collection and agreed disclosure rules

Delivers §2: "the service analyzes them confidentially after a timed collection period. The organizer receives a report under agreed disclosure rules."

- **Agreed rules.** `InitSurvey` takes the full survey definition: questions or walkthrough, deadline, disclosure rules, and the target issue or PR. It stores the definition and outputs `definition_hash` together with the public key, so both are covered by the same attestation.
  - The respondent CLI shows the definition and verifies the hash before encrypting.
  - `Submit` binds each response to `definition_hash`.
  - `Finalize` applies only the stored rules.
  - The organizer can't change questions, deadline or rules after creation.
- **Timed collection.** `Finalize` is called through a NEAR transaction (on-chain mode), which provides a trusted `NEAR_BLOCK_TIMESTAMP`. The module refuses to finalize before the deadline. The organizer's transaction reveals nothing about respondents.
  - `Submit` stays on HTTPS. It is rejected once `Finalize` has set the closed flag. Late submissions are possible until then, which is harmless.
- **Testnet check:** `NEAR_BLOCK_TIMESTAMP` is present and reliable in on-chain runs, and OutLayer issues #32 and #33 (testnet on-chain timeouts) don't block it. If on-chain mode fails, early finalization becomes a disclosed weakness, not a guarantee.

## D11. Evidence over headcount

- Every finding in a report carries an evidence class:
  - **reproducible:** steps the maintainer can run;
  - **evidence-backed:** transcript or error excerpts went into the synthesis;
  - **reported:** a claim with no evidence.
- Counts (responses, merged responses, anchored vs unanchored) are shown as context only, never as the case for a finding.
- The report states its own reliability limits.
- Why: identities are free to create, so counts can be inflated. A reproducible finding is reliable whoever submitted it.
- Later: enclave-verified respondent attributes (e.g. "has used this feature") without revealing identity.

## D12. One organizer account, run by us in GitHub Actions

Replaces "each organizer is the caller" (2026-11-05), which put five setup steps in front of a maintainer's first survey: a funded NEAR account, the OutLayer CLI, a restricted payment key, a NEAR AI key, and that key re-entered per release.

- **One organizer account** (`research.agency.testnet` on testnet) is the caller for every survey. It signs `InitSurvey` and `Finalize`, owns the published submission key, and owns the `nearai` secret row per release. We set these up once per release.
- **Create:** a collaborator with write access comments `/survey` on an issue or PR with a definition. A GitHub Action validates it, checks that its target is that issue, creates the survey and posts the announcement.
- **Finalize:** a scheduled Action finalizes surveys past their deadline and posts the report with its provenance line.
- **Why it's still safe:** the locked module enforces everything regardless of who holds the caller account (no plaintext output, one `Finalize` after the deadline, replay-safe submissions). The account gives availability power only. Respondents pin it in `docs/releases.md` like the release.
- **Costs:** we pay for submissions and model calls, and surveys depend on our Actions being available. Abuse is bounded by GitHub's write-access gate, the same one `private-ai` uses for `/review`.
- The CLI stays GitHub-agnostic; the Actions glue it to GitHub. Development uses the unlocked `discovery-dev` project with `--allow-unlocked`.

## Verified constraints (2026-11-05)

- **The public job log reveals activity.** `GET https://api.outlayer.ai/public/jobs` needs no auth and lists every run with caller account, project, timestamps, duration and instructions used (checked live). With one shared service payment key, the caller is always us, so *who* responded stays hidden. **The number and timing of submissions are public.**
- **Shared `@worker` storage only has `set-worker` and `get-worker`.** It has no atomic writes and no `list-keys` (`sdk/outlayer/wit/deps/storage.wit:111-120`). The per-caller cell has the full set: `set-if-absent(-raw)`, `set-if-equals`, `list-keys`. Under one shared payment key, that cell is effectively one shared cell. This corrects the first research doc, which said `@worker` had atomic ops.
- **Anyone can store a secret bound to our project.** For the `Project` accessor, `store_secrets` only checks that the project exists (`contract/src/secrets.rs:2004-2011`). The module must never trust a secret from a `secrets_ref` the caller chooses. D5 avoids secrets entirely.
- **Encrypted storage keeps an unsalted SHA-256 of each record name** (per the review). Record names must be `mac` outputs so nobody can guess-and-confirm `survey:42:alice.near`.
- **Submit's output hash is published in the attestation.** The output must be constant or randomly salted.
- **No GitHub token in the enclave.** A stored credential would expire and need renewing, and the module would hold more power than it needs. Finalize returns the report as attested output and writes it to public worker storage. The maintainer's agent posts it with its own token, as in D4. Readers verify the report hash against the attestation.

- **OutLayer's GitHub connector** (https://skills.outlayer.ai/github-connector/SKILL.md) acts as the owner. Its policy can make writes wait for owner confirmation (`awaiting_owner`), and it appends an AI-agent marker by default. It covers issues, comments, labels, PRs and gists, but **has no GitHub Projects or sub-issue operations**. The maintainer's agent can post the report and create task issues through it, with human approval built in. Boards (D4) need the maintainer's own GitHub credentials outside the connector.

- **Open OutLayer issues that affect us:** #32 and #33 (testnet: newly uploaded WASM isn't picked up, and requests time out) could block the testnet checks for D5 and D6. #35 (public attestations can be enumerated and expose unsalted input hashes) confirms that inputs and outputs must have high entropy or be salted. Ciphertext inputs are fine.

## Reference: near-research/survey-tool

Reuse: client-side EC01 encryption to a TEE-held key, per-survey key derivation, ciphertext-only storage, signer auth inside the module.

Must change:
- `ReadResponses` returns plaintext responses plus submitter IDs to the creator. Replace it with a TEE-internal synthesis step that outputs only the report.
- Signed transactions and the plaintext `submitter_id` column expose who participated.
- The client gets the encryption public key from operator config. It must fetch it from attested code instead.
- The operator-held `db-api` can insert or drop rows. The TEE must authenticate each stored response itself.
- Calls go through a browser wallet. Agents need a headless path.

## Review corrections (2026-11-05)

- D10 intentionally permits HTTPS submissions after the earliest finalization
  time until collection is closed. The review's proposed strict-cutoff defect
  was withdrawn after checking that decision.
- Near-duplicate grouping affects counts and model weighting only. Every
  response's evidence, repro steps and permissions remain in synthesis and
  publication checks. A matching answer text cannot discard confidential
  identifiers or a contributor's approved phrasing.
- Scheduled finalization includes PRs and closed issues and follows each
  survey's original locked release, rather than the current creation default.
- `report --finalize-tx` verifies the pinned contract execution and reads the
  report from its chain output. HTTPS-only report retrieval is explicitly
  unverified; NEAR AI receipt verification is a separate, unimplemented CLI
  capability.

## Prototype simplification (2026-11-05)

- The CLI uses the shared UTC formatter from `discovery-protocol`; it no
  longer depends on the NEAR AI attestation and WASI HTTP stack.
- The confidential model session is created on the first inference call.
  Rejected, empty and already-completed finalizations need no model key or
  attestation request. Inference still verifies fresh attestation before
  transmitting any response content.
- Duplicate groups, synthesis input and mechanical publication-check indexes
  are prepared once per batch. Retries reuse the same evidence and rules,
  changing only the private rejection feedback. All contributor evidence and
  permissions remain included.

## Verified fixes after product grading (2026-11-05)

- Deterministic interleaving tests reproduced an older response overwriting a
  newer edit. Production response updates now use a bounded ciphertext CAS
  loop, and first submissions use set-if-absent. Replays retain the same
  acceptance reply. Unconditional overwrite remains a test-only helper.
- Offline attestation fixtures reproduced the first-valid-candidate shortcut.
  Every returned model candidate is now checked and retained; duplicate signer
  keys and incomplete recorded GPU-verdict arrays are rejected. The selected
  E2EE key remains mandatory for signed turns. See `crates/nearai/VENDORED.md`
  for provenance, receipt extensions and deployment-policy limitations.
- A crash between receipt and report writes reproduced a receipt/report hash
  mismatch on retry. Receipts are now content-addressed and retrieval follows
  and verifies the released report's receipt hash. Competing losing runs can
  leave unreferenced attestation receipts containing no response plaintext.
- Product-review release checks require every acceptance criterion to have a
  verdict and failures (`not_met` or `mixed`) to carry structured severity.
  `not_assessed` represents missing authorized evidence; it is never inferred
  as passing. Empty product reviews generate that verdict for every criterion
  without calling the model. Incomplete generated reports retry once, then
  withhold. Old reports without severity remain readable by updated clients.

These local tests do not establish a completed live demo or a full concurrency
proof. Submission/finalization interleavings still need a dedicated state
transition review; per-response CAS alone does not serialize the whole survey.
The original concept remains the requirements baseline.

## D13. Freeze collection without leases or a cross-record transaction

A deterministic test reproduced a first submission paused before insertion,
then resuming after finalization had listed and deleted responses. It restored
response data after release. Checking the closed flag before a write cannot
fence a write already in flight.

The next release uses a monotone protocol over OutLayer's single-record CAS:

1. Before writing a new response slot, publish its MAC-derived name into a
   sealed registration list. Register through CAS on a small collection-head
   record. Random, immutable list nodes keep the head bounded; this introduces
   no smaller batch-size limit. Existing response slots are already registered.
2. Finalization closes admission through CAS on that same head. Registrations
   that lose the race cannot publish a response. A crashed registration may
   leave an unused metadata node, but no response content.
3. Freeze every registered slot before synthesis. If an admitted writer beats
   freezing, its response is included; if freezing wins, the writer cannot
   commit. Missing slots receive frozen-empty markers. Repeated finalization
   sees the same frozen contributions and resumes interrupted freezing.
4. After storing the report, replace response contents with sealed empty
   tombstones and the survey secret with a sealed empty marker. Keep these
   markers: deleting them would let a delayed initial insert recreate data.
   The registration list contains only opaque slot names and links.

The slot phase is a byte authenticated as AAD alongside the storage key. The
submission itself remains the encryption plaintext; a wrapper does not reduce
the existing response-size limit. Phase changes and cross-slot ciphertext
swaps fail authentication. Public submission and status wire formats remain
unchanged.

This provides a stable input batch before inference and resumable cleanup
without a clock-based lease or a crashed lock holder. In-flight submissions
that began before closure may be included or rejected according to which CAS
wins; a successful contribution is not silently dropped. Tombstones retain no
answer, signature, wallet identity or private-key bytes. Metadata storage is
retained; this is content erasure, not removal of every record name.

Local regressions cover registration/closure, first-insert/release,
edit/freeze/discard, an admitted write winning against freezing, partial
freezing failure and retry, phase authentication, and response-size preservation.
These tests assume the host's documented CAS and durable storage semantics;
they are not a proof against storage rollback or a fresh testnet verification.
This storage format requires a new locked project, not an in-place migration.

## Local identity and report fidelity (2026-11-05)

- Optional custody anchoring is now exposed through local CLI setup/import and
  explicit `submit --anchored`. Setup does not name a survey; submissions make
  no custody-service calls. Default identities remain per-survey. Local secret
  creation is exclusive with private permissions and never overwrites malformed
  or unreadable keys. See `docs/custody-anchoring.md`.
- Findings carry evidence basis derived from private references to authorized
  answers. Model-assigned basis is overwritten; source references are removed
  before report storage, receipts and publication, and rejected in narrative.
  Unsupported reproducible/evidence-backed labels are downgraded. Failure
  scenarios are now structured, checked for review failures and included in
  publication filtering. Semantic correctness remains a model-audit concern;
  source references are not proof that a claim actually follows from its source.
- Workload approval is now enforced by the module, using configuration hashes
  baked into its binary. The policy is intentionally unconfigured: survey
  creation and real synthesis fail closed until reviewed workload records are
  supplied. No observed hash has been treated as trusted approval. See
  `docs/workload-approval.md` for the evidence required before a new release.

One frozen collection and one released report are enforced. A literal single
physical inference execution is not established: D3 explicitly permits a
regeneration, transport failures can cause uncertain execution, and concurrent
finalizers can do redundant work. No provider idempotency contract was
established in this review. The original concept is unchanged; do not represent
single publication as an exactly-once computation guarantee.
