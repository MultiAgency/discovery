# Releasing the service

How to deploy a locked release and prove it on testnet before collecting
personal data. Use the official
[OutLayer CLI skill](https://skills.outlayer.ai/outlayer-cli/SKILL.md) for
deployment, secrets and payment keys.

Existing projects are immutable: new code and formats ship as a new locked
project. `discovery` refuses a release until it is locked, and a survey
created before the lock never verifies.

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
2. Use a fresh, initialized NEAR account as the release owner and never deploy
   code to it. Keep only the reviewed version, then delete all of the owner's
   access keys.
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
   `{"service": {"project": "<owner/project>", "release": "<wasm sha256>", "organizer": "<account>"}}`,
   then reinstall the CLI. The CLI refuses a malformed entry or an organizer
   that is the project owner. Finish open surveys before replacing it.
5. Pass the synthetic smoke test below before collecting personal data.

Install the official NEAR CLI. Its legacy keychain
(`~/.near-credentials/testnet/<organizer>.json`) holds the organizer key;
`discovery` never handles it:

```bash
cargo install near-cli-rs --version 0.30.1 --locked --no-default-features
```

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
- **Synthetic smoke test against the locked release** (2026-11-06):
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
- **Peak storage calls** are known only from local tests: 61 for 18 stored
  responses and 97 in the worst case, within the 100-call budget below the
  126th call at which testnet runs failed.
