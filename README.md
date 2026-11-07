# Confidential surveys for agents

A maintainer asks for context about an existing GitHub issue. People's personal
agents submit authorized, encrypted responses. After collection closes,
confidential processing releases a report under the disclosure rules, or a
withholding notice, and the maintainer posts it to the issue.

**Existing issue → survey → private responses → permitted report → better requirements.**

Existing agents provide the interface; respondents need no wallet or identity
key.

| Read | For |
|---|---|
| [docs/policy.md](docs/policy.md) | what the service promises: authorization, disclosure, retention, trust, limits |
| [organizer skill](skills/discovery-organizer/SKILL.md) | designing, running and publishing a survey |
| [respondent skill](skills/discovery-respondent/SKILL.md) | verifying a survey and submitting approved answers |
| [docs/release.md](docs/release.md) | deploying a locked release and the deployed smoke test |
| [docs/demo.md](docs/demo.md) | the local demo and the live invitation demo |
| [docs/concept.md](docs/concept.md) | the product concept |

## Readiness

**Live use is not ready.**

- [releases.json](releases.json) registers the locked release below; the CLI
  verifies against it.
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

  In those runs, the locked build's drafts kept stating counts ("one
  respondent") and reusing answer wording, and the count check flagged real
  counts. The release candidate's prompt mostly stops the counts, but
  forbidding quotes still withholds often, because drafts reuse the product's
  own vocabulary from publishable answers. Provider errors were transport failures between this
  machine and NEAR AI; on-chain, such an error ends finalization, which is
  then retried. The logs do not record a build hash: the release-candidate
  rows rest on rebuilding the same source tree after the runs, which gave
  `d6b8f058` again.
- **Still required:** release candidate
  `d6b8f0582e278e32c757456e7d4f78d317a9c217bdd3c113ecc7c8a35445e105`
  (revised prompt and count rule; two clean builds match) goes to a new
  project, because the locked one can't change. Demo surveys allow quotes of
  publishable answers. It goes through the same full-capacity run, lock,
  registration and smoke test. Peak storage calls are known only from local
  tests: 62 for 18 stored responses and 98 in the worst case, below the 126th
  call at which testnet runs failed.

## Quick start

Tested with Rust 1.97.0; the service build needs the `wasm32-wasip2` target.
These checks need no credentials and run in CI:

```bash
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --locked --target wasm32-wasip2 -p discovery-module
```

`scripts/demo.sh` runs the request-to-report cycle locally on invented answers,
with no credentials. It is a simulation (storage, block time and the model are
faked, there are no transactions), not evidence of deployed confidentiality.

For real surveys, install the CLI from a trusted checkout and give your agent
the issue URL and the matching skill from that checkout:

```bash
cargo install --locked --path crates/cli
```

The CLI compiles in the service from this checkout's `releases.json`; no
announcement or option can override its project, build hash or organizer. The
only setting is `DISCOVERY_PAYMENT_KEY`, the published submission key.

| Command | Who | Does |
|---|---|---|
| `prepare init <definition.json>` | organizer | validate; print unsigned creation arguments to sign with `near` |
| `verify <survey> --init-tx <tx>` | both | verify the survey on-chain against the release; show definition and state |
| `submit <survey> --init-tx <tx> <answers.json>` | respondent | verify, encrypt to the verified key, submit |
| `prepare finalize <survey>` | organizer | print unsigned finalization arguments to sign with `near` |
| `report <survey> --finalize-tx <tx>` | both | verify the report on-chain and print Markdown |

`--target OWNER/REPO#NUMBER` makes `verify`, `submit`, `report` and
`prepare init` refuse any other issue. All local input errors are reported
together before any network or paid call.

Source layout: `crates/protocol` holds the shared formats and response
encryption; `crates/module` is the OutLayer service (storage, collection,
NEAR AI inference, publication checks, cleanup); `crates/cli` is the client.
