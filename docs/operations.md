# Operating issue surveys

Start with [the concept](concept.md) and [current service policy](decisions.md).
The CLI is GitHub-independent; Actions publish announcements and reports on
existing issues. A maintainer needs only repository write access, not model or
wallet credentials.

The integration posts survey announcements and shared reports as comments on
the survey's existing target issue. It does not publish individual responses or
turn them into new issues. Any later issue creation is a separate decision by
the maintainer, outside the survey service.

## Prepare a release

The current embedded workload policy is empty. Do not collect responses with
this source or activate it until [workload approval](workload-approval.md) is
complete. The historical registry entry has no active creation flag. Existing
surveys require the compatible earlier CLI and their original locked release;
see [releases](releases.md).

1. Approve the NEAR AI workload, build the WASI module without `diagnostics`,
   review it and deploy it as a new OutLayer project.
2. Keep only the reviewed version, then lock its owner by removing access keys.
   Verify the keyless owner, single active version and shared storage/caller
   behavior on testnet using synthetic data.
3. Add a new `releases.json` entry with network, project, wasm hash, organizer,
   `"locked": true` and `"use": "surveys"`. There must be exactly one creation
   entry. Preserve older entries and hashes until their surveys finish.
4. Configure the organizer's `nearai` secret profile for that project, restricted
   to the reviewed build. No GitHub credential enters the enclave.

OutLayer setup: https://skills.outlayer.ai/outlayer-cli/SKILL.md.
The build must retain its manifest, egress policy and project-bound key path.
Use the same organizer account for HTTPS payment keys and on-chain execution;
changing the caller changes the storage/key namespace. The CLI checks payment-key
ownership and verifies the exact signed call and OutLayer execution on chain.

The organizer account must be separate from the release owner. Deploy with a
separate temporary owner and remove its access keys after review. Local signing
does not replace release locking or model workload approval.

## Official CLI and local key

Organizer commands require `near-cli-rs` 0.30.1, available as `near` on PATH:

```bash
cargo install near-cli-rs --version 0.30.1 --locked --no-default-features
near --version
```

Choose one key source:

- Locally, use the official CLI's legacy keychain at
  `~/.near-credentials/<network>/<organizer>.json`.
- Set `DISCOVERY_ORGANIZER_KEY_FILE` to a standard NEAR access-key JSON file.
- In CI, provide `DISCOVERY_ORGANIZER_KEY` as a canonical `ed25519:<base58>`
  64-byte NEAR keypair. Do not pass its value in command arguments or logs.

The wrapper gives the official CLI the exact contract call through a private
argument file and asks it to save a signed transaction. The CLI handles keys,
nonces, encoding and signatures. No custom transaction encoder or signer is
used. Temporary key copies have mode 0600 in a mode-0700 directory and are
removed immediately after signing. Subprocess terminal output is not parsed or
published. GitHub Actions cache only the installed CLI binary, never keys.

Our RPC adapter submits the resulting signed bytes and polls those same bytes
for final execution. It checks signer, receiver, sole action, method, arguments,
gas, deposit and the pinned OutLayer execution before using its output.

The official OutLayer CLI remains useful for deployment, secrets and payment-key
setup. Its `run` command automatically chooses HTTPS when a payment key is
configured; organizer execution uses the NEAR CLI to keep the on-chain path
explicit and preserve trusted block time.

References:
- https://github.com/near/near-cli-rs
- https://docs.near.org/api/rpc/transactions

## Configure the repository

Set these repository secrets without printing their contents:

| Secret | Purpose |
| --- | --- |
| `DISCOVERY_ORGANIZER_KEY` | Service account's NEAR keypair for local creation/finalization signing |
| `DISCOVERY_PAYMENT_KEY` | Shared submission key owned by that organizer |

The payment key is intentionally published in announcements. Restrict it to the
release project with a small per-call spending cap and monitor its balance.
Keep the service account funded for execution deposits and gas. The restricted
submission key remains owned by that account. Historical report formats require
the earlier CLI; keep their original credentials available until they finish.

## Request and publish

A write collaborator posts `/survey` with one JSON definition block on an
existing issue. `survey.yml` validates it and matches its target before creating
it. PR comments do not create new surveys in this MVP.

`finalize.yml` scans issue comments every 15 minutes, including closed issues,
and follows each announcement's original registered release. Entries using the
older removed report format are excluded and require the earlier CLI.
Only bot-authored announcement documents schedule work. Only the CLI's full
terminal provenance footer completes a survey; report prose never controls
scheduling. Failed surveys are retried on the next run.

The report posts automatically, as requested when the maintainer creates the
survey. The service ends at the shared report. The maintainer reviews its
findings and decides whether to refine the existing issue's requirements.

## Manual CLI path

With the release's `DISCOVERY_NETWORK`, `DISCOVERY_PROJECT`, `DISCOVERY_RELEASE`,
`DISCOVERY_ORGANIZER` and `DISCOVERY_PAYMENT_KEY` configured, plus the organizer
local organizer key configured through one of the sources above:

```bash
discovery organizer create definition.json --target owner/repo#42 --announcement announcement.md
discovery verify <survey> --init-tx <init_tx>
discovery status <survey>
discovery submit <survey> --init-tx <init_tx> answers.json
discovery organizer finalize <survey> --comment report.md
discovery report <survey> --finalize-tx <finalize_tx>
```

Post the generated announcement and report using the maintainer's GitHub
credentials outside the enclave. The verified report command reads the chain's
output and checks signer, contract, project, build, survey and historical release
lock. Without `--finalize-tx`, retrieval is an explicitly unverified HTTPS copy.
Neither command independently verifies the NEAR AI receipt.

## Failures and completion

The deadline is an earliest finalization time, not a strict HTTPS cutoff.
Finalization freezes inputs, stores one report and erases response/key contents.
Retry the same command after interrupted freezing or cleanup. Do not delete
empty sealed markers or registration records: they fence delayed writers.

A withheld report is a successful privacy outcome, with no private reasons.
Network/model errors are incomplete runs and require reconciliation or retry;
response contents do not expire independently. There is no custody API or wallet
request-status layer in organizer execution.

If submission is uncertain, the CLI retains the signed JSON and prints its path.
Use the same operation, definition/profile and pinned release settings with
`--resume-signed <path>` to query that exact transaction, without signing or
broadcasting a replacement:

```bash
discovery organizer finalize <survey> --resume-signed /private/path/signed.json --comment report.md
```

Actions retain only uncertain `signed.json` files as a seven-day recovery artifact;
they never upload private-key files. New paid Actions are blocked while an
unexpired recovery artifact exists. Download it and reconcile the transaction
promptly, then remove the artifact to unblock automation. Local commands need
the same reconciliation before starting another paid attempt. Signed artifacts authorize an already approved public organizer call;
handle them as operational recovery material. Successful calls erase the local
artifact automatically. A resumed artifact remains until the operator removes
it after reconciliation.

If the transaction was never accepted, inspect it before rebroadcasting those
same signed bytes using the official CLI. Do not rerun creation/finalization
with a new signature merely because a response timed out. Known final execution
failures can be investigated by their printed transaction hash before deciding
whether a new paid attempt is appropriate.

Before claiming the demo complete, run the complete Actions/CLI path on testnet
with synthetic identifiers. Check report provenance, absent identifying details,
closed status, no public key and refusal of further submissions. Record the
actual criterion or assumption the maintainer changed. Local checks in the root
README and older spike results do not establish that deployed outcome.
