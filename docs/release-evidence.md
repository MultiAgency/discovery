# Release evidence

Historical record: what each release and candidate showed on testnet, newest
last. How to release is in [release.md](release.md); the current release is the
one [releases.json](../releases.json) registers. Live use waits for a release
to pass the smoke test after its lock.

Release `4adb3f91` (last entry) is the first to require E2EE v3, refuse
contract-mediated executions, verify complete creation inputs from signed
transaction arguments, and carry the [clean bootstrap proof](release.md#clean-bootstrap-proof). The earlier
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
  responses and 97 if every slot races, within the 100-call budget, below the
  126th call at which testnet runs failed. Registrations contending with
  closing (7 lost attempts) raise one finalizer to 111, which exceeds the budget
  but stays below the failure; a separate module test reproduces that case
  (`combined_close_contention_and_slot_races_reach_111_storage_calls`). A competing finalizer's extra calls follow the stored report, so a
  failure leaves only cleanup for a retry.
