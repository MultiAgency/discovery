# Releasing the service

How to deploy a locked release and prove it on testnet before collecting
personal data. Use the official
[OutLayer CLI skill](https://skills.outlayer.ai/outlayer-cli/SKILL.md) for
deployment, secrets and payment keys.

New code and formats ship as a new locked project. `discovery` requires a clean
bootstrap proof as well as a lock: a snapshot of the remaining versions cannot
reveal an earlier version that was removed after planting state.
Initialization receipts must follow the verified lock. What earlier releases
showed is in [release evidence](release-evidence.md).

## Deployed service and source

`releases.json` pins the deployed service: its project, build hash, organizer
and bootstrap proof. The CLI compiled from any commit uses only that pin, so a
checkout can carry a newer, compatible CLI while respondents still use the
deployed build.

The module source in a checkout is not necessarily that build. Module or
protocol changes after a release are unreleased work: they produce a different
hash and run nowhere until a new locked project is bootstrapped, smoke-tested
and registered.

| Deployed release | Reviewed source that reproduces it |
|---|---|
| `4adb3f91082e9fcdbeb8032b9b38fe67e6adf808d2f20a21964919f4b609bc5a` | commit `5ea519f` (step 1 below rebuilds it) |

An invitation's checkout commit chooses the CLI respondents install, not the
service; it may be newer than the reviewed source above. To audit the service,
rebuild the listed commit and compare its hash with the pin.

Builds currently embed toolchain installation paths. The documented source
reproduced `4adb3f91` locally with default stable Cargo on Rust 1.97.0. An
explicit `+1.97.0` installation of the same compiler produced a different hash
solely because embedded toolchain paths differed. Record `rustc -vV`,
`cargo --version`, the toolchain location, build flags and artifact hash with each
release. Portable hash reproduction needs a hermetic build with path remapping;
a matching compiler version alone is insufficient.

## Unreleased hardening

`releases.json` still registers `4adb3f91`; the checks below require a new locked
service release and post-lock smoke test. They do not retrofit existing surveys.

- Every mechanically valid draft with responses receives one combined privacy
  and factual-support audit, including publishable-only responses. Missing or malformed verdicts
  reject the draft. This is best-effort model review, not proof of truth.
- Proposed acceptance criteria must cite valid, authorized answers, as findings
  already do. Private citations are checked and removed before publication.
  The model is told it lacks the issue body; proposals require maintainer review
  against the actual issue rather than claims about known existing criteria.
- New definitions cannot contain a withheld term in their report title.

The current CLI can still verify and render the registered release's unchanged
report format. Its **source basis** labels describe cited answers, not confirmed
findings. Its local payment-key owner checks and fixed error classifications
also apply when using the current service.

Before registering the new module, repeat the full-capacity smoke test: auditing
publishable-only batches adds work to a path that previously skipped the audit.
The maximum remains three drafts plus three audits in one 180-second job. Record
fidelity and disclosure review of the generated reports, not just valid JSON.

Public payment/admission abuse and independent response expiry remain open.
The 18-registration limit and indefinite retention policy have not changed.
A discard path needs a defined public outcome, organizer authority, trusted
time, provenance verification, and the existing delayed-write/cleanup fences;
changing storage or weakening the release pins is not a substitute. Keep the
pilot controlled while those decisions and an independent participant run are
outstanding.

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
   then reinstall the CLI. The CLI refuses a missing or malformed entry, an
   owner that is not a named subaccount, and an organizer that is the project
   owner. It pins exactly one release; reports from an earlier release are
   verified with the CLI built from that release's commit. Record the frozen
   source's commit in [Deployed service and source](#deployed-service-and-source).
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
invented answers in [fixtures](../fixtures). Every `create`,
`early`, `finalize` and `refinalize` is a paid call signed by the organizer.
[scripts/smoke.sh](../scripts/smoke.sh) saves each signed transaction and
records its hash in `~/.discovery-smoke/<name>/transactions.log` before
broadcasting. Re-running a step whose outcome is unknown re-broadcasts that same
transaction rather than signing a new one; `smoke.sh status <name> <label>`
reconciles it. Record OutLayer job durations alongside.

```bash
export DISCOVERY_PAYMENT_KEY=<organizer:nonce:secret>
s=scripts/smoke.sh; a=fixtures
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
8. **Record the evidence** in [release-evidence.md](release-evidence.md):
   transactions, outcomes and job times.

Local tests already establish encryption binding, deduplication, admission and
freeze fences, publication checks, withholding, single-report storage, cleanup
markers and confidentiality-safe errors, all against storage and model doubles.
A deployed run adds evidence for the storage behavior it exercises (not CAS
semantics in general), keystore and attestation behavior, model output quality
and limits, job timing, cost, and on-chain provenance as the CLI sees it.

`scripts/test-smoke.sh` checks the script's transaction recovery offline, with
`near` and `curl` stubbed.
