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
| [docs/release-evidence.md](docs/release-evidence.md) | what past releases showed on testnet |
| [docs/demo.md](docs/demo.md) | the local demo and the live invitation demo |
| [docs/concept.md](docs/concept.md) | the product concept |

## Readiness

**A first real survey is open on
[MultiAgency/discovery#1](https://github.com/MultiAgency/discovery/issues/1).**

- [releases.json](releases.json) registers locked release `4adb3f91` with a
  verified clean bootstrap. Its post-lock synthetic smoke test passed,
  including deployed NEAR AI E2EE v3.
- The deployed service and this source can differ: see
  [deployed service and source](docs/release.md#deployed-service-and-source).
- Before inviting anyone: render the [invitation](docs/invitation.md) with
  `scripts/invitation.sh` and have a teammate rehearse it from a clean machine
  ([runbook](docs/demo.md#runbook)).
- The first `discovery verify` or `submit` on a machine checks the release's
  bootstrap proof (about 43 archival RPC calls; tens of seconds on FastNEAR's
  free tier, seconds with an optional `FASTNEAR_API_KEY`). The passed proof is
  remembered in `~/.cache/discovery/` for that exact release, so later
  commands make 6 archival RPC calls. Stage progress and fixed retry/backoff
  notices go to stderr; verification JSON on stdout stays unchanged. Anyone
  who can write that folder could also replace the CLI.
- The testnet transactions, run results and local experiments are in
  [release evidence](docs/release-evidence.md).

## Quick start

Tested with Rust 1.97.0; the service build needs the `wasm32-wasip2` target.
These checks need no credentials and run in CI:

```bash
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --locked --target wasm32-wasip2 -p discovery-module
```

`scripts/demo.sh [OUT_DIR]` runs the request-to-report cycle locally on
invented answers, with no credentials, and can save its transcript and both
public reports. It is a simulation (approval, storage, block time and the model
are faked, there are no transactions), not evidence of deployed
confidentiality. `scripts/test-smoke.sh` and `scripts/test-invitation.sh` test
the live tooling offline; `scripts/test-demo.sh` checks the demo launcher's
output handling.

For real surveys, install the CLI from a trusted checkout and give your agent
the issue URL and the matching skill from that checkout:

```bash
cargo install --locked --path crates/cli
```

The CLI compiles in the service from this checkout's `releases.json`; no
announcement or option can override its project, build hash or organizer. The
only required setting is `DISCOVERY_PAYMENT_KEY`, the published submission key;
`FASTNEAR_API_KEY` optionally raises the RPC rate limit without changing the
endpoint.

| Command | Who | Does |
|---|---|---|
| `prepare init <definition.json>` | organizer | validate; print unsigned creation arguments to sign with `near` |
| `verify <survey> --init-tx <tx>` | both | verify the survey on-chain against the release; show definition and state |
| `submit <survey> --init-tx <tx> <answers.json>` | respondent | verify, encrypt to the verified key, submit |
| `prepare finalize <survey>` | organizer | print unsigned finalization arguments to sign with `near` |
| `report <survey> --finalize-tx <tx>` | both | verify the report on-chain and print Markdown |
| `invitation <survey> --init-tx <tx> --commit <sha> --page <url>` | organizer | verify an accepting survey and print its invitation page; run via `scripts/invitation.sh` |

`--target OWNER/REPO#NUMBER` makes `verify`, `submit`, `report`, `invitation`
and `prepare init` refuse any other issue; `invitation` requires it. Each
command checks its local inputs before any network or paid call and stops at
the first problem.

Source layout: `crates/protocol` holds the shared formats and response
encryption; `crates/module` is the OutLayer service (storage, collection,
NEAR AI inference, publication checks, cleanup); `crates/cli` is the client.
`fixtures/` holds the invented survey and answers shared by the demo, tests
and smoke test.
