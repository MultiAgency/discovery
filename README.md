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

- [releases.json](releases.json) registers locked release `be2b43ef` on
  `test2.research.agency.testnet/discovery`. It passed the synthetic smoke
  test, but its reports withhold most small surveys.
- A revised release is being validated on a new project before it replaces
  that one.
- The testnet transactions, run results and local experiments are in
  [Release evidence](docs/release.md#release-evidence).

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
