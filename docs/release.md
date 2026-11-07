# Releasing the service

How to deploy a locked release and prove it on testnet before collecting
personal data. Use the official
[OutLayer CLI skill](https://skills.outlayer.ai/outlayer-cli/SKILL.md) for
deployment, secrets and payment keys.

New code and formats ship as a new locked project. `discovery` requires a clean
bootstrap proof as well as a lock: a snapshot of the remaining versions cannot
reveal an earlier version that was removed after planting state. Old testnet
projects cannot supply this proof and are not registered by the current CLI.
Initialization receipts must follow the verified lock.

## Setup

1. Freeze the source, then build and review the module and deploy those exact
   bytes to a new project. Build outside the shared `target/`, hash the copy you
   upload, and keep the source it came from, including untracked files. Any
   change to the module's source or manifests, including dependency lists,
   changes the hash.

   ```bash
   dir=~/.discovery-release; mkdir -p "$dir"
   CARGO_TARGET_DIR="$dir/target" cargo build --release --locked --target wasm32-wasip2 -p discovery-module
   cp "$dir/target/wasm32-wasip2/release/discovery-module.wasm" "$dir/release.wasm"
   shasum -a 256 "$dir/release.wasm"     # the release hash; upload this file
   git ls-files -co --exclude-standard -z | tar --null -T - -czf "$dir/source.tgz"
   ```
2. Bootstrap a fresh release owner and project as described below. Upload the
   WASM using a separate account: the release owner's only transactions must be
   project creation followed by deleting its sole key. Record all three
   bootstrap transaction hashes. Never deploy account code or add another
   version, even temporarily.
3. Use a separate funded organizer account. Give it a `nearai` secrets profile
   holding `NEARAI_API_KEY`, restricted to the reviewed build. Create the
   shared submission payment key under the same account, since the caller
   selects the storage namespace: the key must start with `<organizer>:`.
   `outlayer` 0.2.1 cannot restrict a key to a project or cap it per call, so
   anyone holding it can spend its whole balance. Keep the balance small and
   monitor it. It is published; the organizer's NEAR key is not. Also set a
   [spend limit](https://docs.near.ai/api-reference/workspaces/update-api-key-spend-limit.md)
   on the NEAR AI API key itself.
4. Set the service in `releases.json`:

   ```json
   {"service": {
     "project": "<owner/project>",
     "release": "<wasm sha256>",
     "organizer": "<account>",
     "bootstrap": {
       "owner_seed_block": "<recent finalized block hash>",
       "owner_init_tx": "<parent creates fresh owner>",
       "project_init_tx": "<owner creates reviewed project>",
       "lock_tx": "<owner deletes its sole key>"
     }
   }}
   ```
   then reinstall the CLI. The CLI refuses a malformed entry or an organizer
   that is the project owner, and requires all bootstrap pins. Existing public
   reports can be retrieved using their original CLI; the hardened CLI does not
   authenticate legacy releases without this proof.
5. Pass the synthetic smoke test below before collecting personal data.

Install the official NEAR CLI. Its legacy keychain
(`~/.near-credentials/testnet/<organizer>.json`) holds the organizer key;
`discovery` never handles it:

```bash
cargo install near-cli-rs --version 0.30.1 --locked --no-default-features
```

## Clean bootstrap proof

Choose a recent finalized block and save its hash as `owner_seed_block`. Derive
the owner's first account label from that hash and a parent you control: take
the first 32 lowercase hexadecimal characters of SHA-256 over the UTF-8 text
`discovery:bootstrap-owner:v1:<owner_seed_block>:<parent>`, without a trailing
newline. The full owner is `<label>.<parent>` and must fit NEAR's account-name
limit. For example, after obtaining the block hash:

```bash
bootstrap_parent=release-parent.testnet
bootstrap_seed='<recent-finalized-block-hash>'
bootstrap_label=$(printf 'discovery:bootstrap-owner:v1:%s:%s' "$bootstrap_seed" "$bootstrap_parent" | shasum -a 256 | cut -c 1-32)
printf '%s.%s\n' "$bootstrap_label" "$bootstrap_parent"
```

The parent creates that account in a direct transaction with exactly
`CreateAccount`, funding via `Transfer`, and one full-access `AddKey`, in that
order. Its creation receipt must land within 32 block heights after the seed.
Save the transaction as `owner_init_tx` and wait for finality before the owner
signs anything. If creation misses the window, start again with a new seed and
owner. Faucet or factory function calls, implicit accounts, multiple keys and
account contracts do not qualify for this proof.

The owner's first transaction calls `outlayer.testnet.create_project` directly,
with the intended project name and the reviewed `WasmUrl` source. Save it as
`project_init_tx`, wait for finality, then have the owner send a direct
transaction to itself containing only `DeleteKey` for that same key. Save it as
`lock_tx` and wait for finality. Keep these steps in separate blocks. Do not
send any other owner transactions, including transactions with a later nonce
that might be waiting for execution when the key is deleted. Uploads, funding
and organizer setup use other accounts.

Verification walks every ancestor block from owner creation through the seed,
checking all account changes, including intermediate changes within a block.
Only the pinned creation receipt may change this owner. Together with the
unpredictable block-derived name, this excludes earlier account lifetimes and
their delayed receipts, assuming SHA-256 preimage resistance at 128 bits.

Verification also checks consecutive owner nonces, the initial block's key
changes, and key state immediately before and during the lock receipt's block.
These checks reject hidden owner activity and higher-nonce transactions already
accepted before key deletion. They bind the created project's UUID and source
to the registered project at survey and report execution. Missing or incomplete
archival evidence is a verification failure.

The proof relies on OutLayer scoping storage and project-bound encryption keys
by the project's on-chain UUID, not its `owner/name`: storage is keyed by
`(project_uuid, account_id, key)` ([projects](https://app.outlayer.ai/docs/projects),
[storage](https://app.outlayer.ai/docs/storage)), and "a project deleted and
created again under the same name has new keys, and cannot open what the old
one sealed" ([encryption keys](https://app.outlayer.ai/docs/encryption-keys)).
The [published contract source](https://github.com/out-layer/outlayer/blob/main/contract/src/projects.rs#L72-L74)
increments a counter before formatting each UUID. Verification relies on the
deployed contract preserving this behavior. So a deleted and
re-created owner or project cannot carry planted state into the release. A
transferred project keeps its UUID, so one moved onto the release's name fails
the UUID binding, and accepting a transfer would need an owner transaction the
proof forbids. The manifest's `callers` block matches the worker's schema; the
worker refuses a run whose manifest does not parse and checks the block before
any key derivation or execution (`worker/src/callers.rs`).

The module manifest allows direct NEAR calls and HTTPS and denies contract
relays and meta-transactions. Its NEAR AI requests require E2EE v3; request AEAD
authenticates the reply key and field label according to the
[official inference proxy](https://github.com/nearai/inference-proxy/blob/53d504156d38a8f951933e8aa8d23b3a0f70a4df/src/encryption.rs).
The public [E2EE guide](https://docs.near.ai/cloud/guides/e2ee-chat-completions)
still describes v2. Source support does not prove that every serving endpoint
has deployed v3: verify compatibility with invented data in the smoke test.
Never work around a failure by downgrading encryption.

## Synthetic smoke test

Run it once per release, on testnet, after the owner is locked, with only the
invented answers in [scripts/synthetic](../scripts/synthetic). Every `create`,
`early`, `finalize` and `refinalize` is a paid call signed by the organizer.
[scripts/smoke.sh](../scripts/smoke.sh) saves each signed transaction and
records its hash in `~/.discovery-smoke/<name>/transactions.log` before
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
$s workload 18 ~/.discovery-smoke/workload # 7: the full 18 registrations
$s create load 30
for f in ~/.discovery-smoke/workload/workload-*.json; do $s submit load "$f"; done
$s finalize load                           #    after its deadline; time the job
$s refinalize load
$s check load
```

1. **Create and verify.** `create` writes a fresh definition (deadline in
   minutes, quotes of publishable answers allowed), records the creation transaction, checks
   the module output and runs `discovery verify`, which must show
   `accepting_responses: true`. `early` must return "the collection period has
   not ended".
2. **Submit and retry.** Three distinct responses mix both tiers and all three
   bases, with context-only identifiers. One is submitted a second time.
3. **Finalize after the deadline.** Submit once more after the deadline (still
   accepted), then finalize. A `{"error": ...}` output stops the script; run
   `finalize` again to retry. A fully final on-chain failure is recorded in
   `<label>.failure.json`; fix its cause and rerun the step for a fresh
   transaction. Unknown or non-final outcomes keep the original signed
   transaction for reconciliation and rebroadcast.
4. **Retrieve and verify.** `discovery report` must verify the transaction and
   release. The script rejects fixture text, identifiers, source labels,
   respondent counts (the module's rule) and rejection details. Check by hand
   for semantic counts the rule misses ("two humans") and that at least one
   acceptance-criterion change or corrected assumption is grounded in the
   answers.
5. **Check completion.** `check` requires `closed: true`, `key_present: false`
   and "survey is closed" for a new submission. `refinalize` must return the
   identical output without inference.
6. **Context-only handling.** A survey with only `context_only` answers and no
   phrasing. Either a withholding notice or a report that passes manual review
   for leaks is acceptable; anything else blocks release. A clean report shows
   safe handling of that sample, not deployed withholding, which only local
   tests establish.
7. **Time the admitted workload.** `workload 18` writes 18 distinct ~4 KB
   responses, the full registration limit. Finalize them, refinalize, and
   record execution time and headroom. If it fails or lacks headroom in the
   180-second job, lower `REGISTRATIONS` in a new release. Collect no personal
   data until the full limit finalizes.

Local tests already establish encryption binding, deduplication, admission and
freeze fences, publication checks, withholding, single-report storage, cleanup
markers and confidentiality-safe errors, all against storage and model doubles.
A deployed run adds evidence for the storage behavior it exercises (not CAS
semantics in general), keystore and attestation behavior, model output quality
and limits, job timing, cost, and on-chain provenance as the CLI sees it.

`scripts/test-smoke.sh` checks the script's transaction recovery offline, with
`near` and `curl` stubbed.

## Release evidence

What each release and candidate showed, newest last. Live use waits for a
released candidate to pass the smoke test after its lock.

Release `4adb3f91` (last entry) is the first to require E2EE v3, refuse
contract-mediated executions, verify complete creation inputs from signed
transaction arguments, and carry the clean bootstrap proof above. The earlier
runs do not establish those guarantees, and the current CLI does not register
their projects.

- **Count check:** a respondent count in any public report field (for example
  "two human respondents", "most responses") is a publication-check violation
  in `crates/module/src/checks.rs`, retried and then withheld like any other.
  It catches phrasing, not every semantic count. An earlier build (`286161a4`)
  lacked it and released a report stating counts.
- **Locked on testnet:** build
  `be2b43ef74ed103e81067e715c425766058291b79f10777b2f835143cd9f56a6`, which
  contains the count check, is the only version of
  `test2.research.agency.testnet/discovery`. The owner's last access key was
  deleted in tx `4EBFKYzTCSveqo3jHGARc5dujZz8Mu8r7mVqFM8Kfsrw`, and the owner
  has no contract code. Before the lock, a synthetic full-capacity run on that
  build showed:
  - All 18 registrations filled with near-maximum responses. A 19th distinct
    response got "survey is full", and a resend of an admitted one was
    accepted.
  - The first finalization failed on a model-provider error and stored
    nothing. The retry released a report in 73 s of the 180 s limit, which
    is useful headroom for that run, not a guarantee for every synthesis.
  - The report passed the public checks (no planted identifiers, six-word
    quotes, source labels or respondent counts).
  - Refinalizing returned the identical report in 8 s. The survey then showed
    closed with its key gone, and refused a late submission.
- **Synthetic smoke test against locked build `be2b43ef`** (2026-11-06):
  - `discovery verify` and `report` accepted the release and every report
    below. Early finalization was refused, and an identical retry and an
    after-deadline submission were accepted.
  - **main** (3 mixed-tier responses): **withheld**, because every draft failed
    the publication checks (tx `EgT3B2EUub9Zv8ipgoCaaNqxzf1L4TX4cPjCWKNHxE1E`).
    The reasons stay private, so the cause is unknown. Refinalize returned the
    identical report, and the survey closed with its key replaced.
  - **quiet2** (context-only, no phrasings): released with no findings (tx
    `4rU8xsdjr2pgzsC3tZL9ByxZyj9LmEZAShyqyNyw4v9G`).
  - **load** (all 18 registrations; a 19th distinct response got "survey is
    full"): released in 34 s with one grounded acceptance-criterion change (tx
    `7M1v1ahG6D8Pz1bGiabrCJEi5jgQMQzigq3okLdwHY7`). Refinalize returned the
    identical report, and the survey closed with its key replaced.
  - Every public report passed the smoke test's checks.
- **Failure modes reproduced locally:** main's rejection reasons are private,
  so its actual cause is unknown. Replaying main's three answer files through
  the same model and checks reproduces likely causes. These are synthetic
  batches of 10 runs per row, run locally against the live model with the
  `live_synthesis` test in `crates/module/src/finalize.rs`; they are not
  general release rates:

  | Code | Quotes of publishable answers | Released | Withheld | Provider error |
  |---|---|---|---|---|
  | locked build `be2b43ef` | forbidden | 2 | 8 | 0 |
  | locked build `be2b43ef` | allowed | 5 | 4 | 1 |
  | release candidate `d6b8f058` | allowed (demo default) | 9 | 0 | 1 |
  | release candidate `d6b8f058` | forbidden | 5 | 4 | 1 |
  | release candidate `8b3e8333` | allowed (demo default) | 9 | 1 | 0 |
  | release candidate `214ed628` | allowed (demo default) | 9 | 1 | 0 |
  | release candidate `214ed628` | forbidden | 6 | 4 | 0 |

  In those runs, the locked build's drafts kept stating counts ("one
  respondent") and reusing answer wording, and the count check flagged real
  counts. The release candidate's prompt mostly stops the counts, but
  forbidding quotes still withholds often, because drafts reuse the product's
  own vocabulary from publishable answers. Provider errors were transport failures between this
  machine and NEAR AI; on-chain, such an error ends finalization, which is
  then retried. The logs do not record a build hash: the release-candidate
  rows rest on rebuilding the same source tree after the runs, which gave
  `d6b8f058` again.
- **Release candidate `d6b8f058`, full-capacity run on
  `test3.research.agency.testnet/discovery`** (unlocked; test driver
  definition with quotes forbidden and one withheld term): 18 of 18
  near-maximum responses accepted, a 19th distinct one refused as full, a
  resend accepted. Finalization **withheld** the report after 117 s of the 180 s
  limit (tx `5W7EKWxGdxXYN5HJBy4ehdo1MbYA5uaYt8Q7c5ewEBjJ`); refinalize returned
  the identical output in 8 s (tx `A1WXnbhhjwzKWQPQfub63V2hpqsh6Tx3aJ6Xww5uzeeh`),
  and the survey closed with its key gone. It does not show how the demo
  setting (quotes allowed) behaves at full capacity.
- **Release candidate `214ed628`, full-capacity run on
  `test3.research.agency.testnet/discovery`** (unlocked, sole version; quotes of
  publishable answers allowed, as in the demo; one withheld term): 18 of 18
  near-maximum responses accepted, a 19th distinct one refused as full, a
  resend accepted; status reported `full: true` while open. Finalization
  **released** a report with 7 findings and 3 acceptance-criteria changes in
  100 s of the 180 s limit (tx `2Ef2yyA41JFDvZK3zQmVCCcrEgGNCY64dk6q6fhGZQeS`).
  It passed `smoke.sh`'s public checks and every planted-identifier, label and
  count check; its only six-word matches are quotes of publishable answers,
  which this survey allows. Refinalize returned the identical output (tx
  `5BHky2krnZivxva2SK2XY7NLhg6JUQFtCXzZCUBQMqrv`); the survey then showed
  closed with its key gone and refused a late submission.
- **Release `4adb3f91`, registered** (2026-11-07). Bootstrapped on the seed
  rule from a frozen source snapshot: seed block
  `9soPUrPWRTYQJp6JcVeBHzS26apTLeQ8TmuxDWqUR6f7`; owner
  `ffff80798e85168e51e6f47b185a6b7c.research.agency.testnet` created 8 blocks
  later (tx `4HA6aRC2Lx9d6TF29tTeP46QKpxBNoUZZGc571eJU49f`); project UUID
  `p0000000000000111` (tx `7URjvSQ1v5H8CVCEFMTZDarZ7xzUmQaxEXHDbCFzA1t2`);
  locked (tx `H82DefwCNra9QEBPHDbQBL41BqJhs6PPSLphn76UTcny`).
  - E2EE v3 was first confirmed live on an unregistered scratch version of the
    previous candidate (tx `BdcrgMYHBNRRn5EDNdgJo9jefLSnjixxU8baBuiGdeFD`).
  - Pre-lock full capacity (quotes allowed): 18 accepted, a 19th distinct
    response refused, released on the first attempt in 27 s with 6 findings
    (tx `C9MNaG55dd6ECWFg2D4fa8sEbJPj6cfqQComGL7L4S5d`); identical refinalize;
    cleanup confirmed. Its six-word matches all quote publishable answers.
  - Post-lock smoke test passed: `discovery verify` accepted the bootstrap
    proof; early finalization refused; an identical retry and an
    after-deadline submission accepted. **main** released with three grounded
    acceptance-criteria changes (tx `AG2oLKmRE7w4GKgaKg5QMmwy9YeDZiyfs4aApL4bvuKA`);
    **quiet** (context-only, no phrasings) released a notice-only report (tx
    `BRXT6CAzftiTQeZRyfUvQg33XYTLDmZEsqSbNZTphpQU`); **load** filled all 18
    places, refused a distinct 19th and released in 46 s (tx
    `ASWPp4n8J2CNYB1RKCMtixNUBjFQfcxsRs3NiDxAtMzm`). Refinalize returned
    identical reports, every report passed the public checks, and each survey
    closed with its key replaced.
  - One finalize (tx `6mLVaoqYLhV4wdXXz2z5SXpxK8oycKjvF43Weg5cnSN8`) completed
    in the service but its OutLayer result was not delivered: the transaction
    is final with a null result ("Execution promise failed") and the deposit
    refunded. A fresh finalize returned the stored report.
- **Peak storage calls** are known only from local tests: 61 for 18 stored
  responses and 97 if every slot races, within the 100-call budget below the
  126th call at which testnet runs failed. Registrations contending with
  closing (7 lost attempts) raise one finalizer to 111, which exceeds the budget
  but stays below the failure; the module's budget test does not cover that
  case. A competing finalizer's extra calls follow the stored report, so a
  failure leaves only cleanup for a retry.
