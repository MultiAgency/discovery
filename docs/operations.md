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
surveys can still be finalized on their original locked release.

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
changing the caller changes the storage/key namespace.

## Configure the repository

Set these repository secrets without printing their contents:

| Secret | Purpose |
| --- | --- |
| `DISCOVERY_ORGANIZER_KEY` | Organizer's NEAR signing key for creation and finalization |
| `DISCOVERY_PAYMENT_KEY` | Shared submission key owned by that organizer |

The payment key is intentionally published in announcements. Restrict it to the
release project with a small per-call spending cap and monitor its balance.
Keep organizer credentials, payment keys and `nearai` profiles available for
older pending surveys. The workflows support historical releases with the same
organizer key; a different organizer requires separately configured credentials.

## Request and publish

A write collaborator posts `/survey` with one JSON definition block on an
existing issue. `survey.yml` validates it and matches its target before creating
it. PR comments do not create new surveys in this MVP.

`finalize.yml` scans comments every 15 minutes, including closed issues and
historical PRs, and follows each announcement's original registered release.
Only bot-authored announcement documents schedule work. Only the CLI's full
terminal provenance footer completes a survey; report prose never controls
scheduling. Failed surveys are retried on the next run.

The report posts automatically, as requested when the maintainer creates the
survey. The service ends at the shared report. The maintainer reviews its
findings and decides whether to refine the existing issue's requirements.

## Manual CLI path

With the release's `DISCOVERY_NETWORK`, `DISCOVERY_PROJECT`, `DISCOVERY_RELEASE`,
`DISCOVERY_ORGANIZER` and `DISCOVERY_PAYMENT_KEY` configured, plus the organizer
signing key for organizer commands:

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
Network/model errors are incomplete runs and require retry; they do not cause
response contents to expire independently. Check a timed-out NEAR transaction
before resubmitting to avoid an unnecessary charged execution.

Before claiming the demo complete, run the complete Actions/CLI path on testnet
with synthetic identifiers. Check report provenance, absent identifying details,
closed status, no public key and refusal of further submissions. Record the
actual criterion or assumption the maintainer changed. Local checks in the root
README and older spike results do not establish that deployed outcome.
