# Spike results: OutLayer testnet, 2026-11-05

Project `spike1.research.agency.testnet/spike` (uuid `p00000000000000e5`). Service account (caller) `research.agency.testnet`. Build v1 `8a6ef8e2…61f8`.

| D5/D6/D10 check | Result | Evidence |
|---|---|---|
| `NEAR_BLOCK_TIMESTAMP` present in on-chain project runs (D10) | **Pass** | `probe` → `block_timestamp: 1793868735578823055`, `block_height: 276319407` |
| Project-bound encryption key and sealed storage work on-chain | **Pass** | `mac_probe` stable (`2c7c1893…e8ad`); sealed record written and read back |
| Randomness in the TEE; `set-if-absent` protects the survey keypair (D5 check 5) | **Pass** | `init_keypair` ×2 → `created: true`, then `false`; same public key `6b634499…3bef` |
| Any version the owner adds shares the key and storage | **Confirmed (the threat D5 closes)** | v2 `fe5e40b1…b54b`, added inactive, ran via `version_key`. It had the same `mac_probe` and read v1's sealed record |
| A removed version can't be run (D5 check 2) | **Pass** | after `remove_version`: `Smart contract panicked … execution.rs:1005 Version not found` |
| A keyless owner's project still runs (D5 check 1) | **Pass** | release project `rel1.research.agency.testnet/discovery` (owner locked, no access keys) ran `init_survey` and `finalize`; see the locked-release section |
| Same key and storage over HTTPS (payment key owned by the service account) as on-chain (D5 check 3) | **Pass** | HTTPS `probe` (call `068cae52…`): `signer: research.agency.testnet`, same `mac_probe`, read the on-chain record (`probe_record_inserted_now: false`) |
| No trusted time over HTTPS (D10 needs on-chain `Finalize`) | **Confirmed** | HTTPS `probe`: `block_timestamp: ""`, `block_height: ""` |
| Public job log shows only the service account (D7) | **Pass, with a caveat** | `testnet-api.outlayer.ai/public/jobs`: `user_account_id: research.agency.testnet`, project, `is_https_call`, `payment_key_nonce`, timing, `instructions_used`. **Instruction counts vary by operation and input size** (`init_keypair` ~2.53M vs `probe` ~126K). Pad ciphertexts to fixed size buckets |
| Custody `sign-message` (NEP-413) verifies in the module (D6) | **Pass** | testnet wallet `ce145b9b…6a84` signed `anchor:spike-local-key-P` for recipient `discovery`; HTTPS `verify_nep413` (call `37edfb35…`) → `signature_valid: true`, `implicit_account_matches_key: true`. Tampered message (call `06205924…`) → `signature_valid: false` |
| Custody signing absent from the public job log (D6) | **Pass** | signed at 2026-11-05T09:08:56Z; `public/jobs` window 06:11–09:09Z has 0 entries mentioning the wallet. OutLayer's front end still sees the request (disclosed in D6) |
| NEP-413 verification, ed25519 and sealed storage fit in the module (D6) | **Pass** | 190 KB wasm; `verify_nep413` ~2.7M instructions (~0.4 s); `init_keypair` ~2.5M; default limit is 1B |

Operational notes:
- OutLayer CLI 0.1.2 `deploy` timed out (retired API address). `create_project`, `add_version`, `remove_version` and `request_execution` work fine called directly with `near` CLI. The CLI was updated to 0.2.1.
- Over HTTPS, `predecessor` is empty. Use only `caller: "signer"` keys.
- Testnet USDC: `storage_deposit` then `mint` (no deposit) on `usdc.fakes.testnet`. The payment key was topped up with $2.
- An on-chain run costs about 0.0012 NEAR. Unused deposit is refunded.
- On-chain `input_data` is public in contract event logs, as expected. Inputs must be ciphertext or non-sensitive.

## Module dev deploy (`spike1.research.agency.testnet/discovery-dev`)

- Active build `14751fce…6556` (307 KB). `init_survey` on-chain created survey `c69d767e…6a20`, key `26091bc3…cc26`.
- `discovery status` and `submit` over HTTPS: three submissions accepted (respondent A twice, B once). The native tests check that A's two land in one slot. No operation reveals the count by design; `Finalize` will confirm it on-platform.
- Rejected on the live module: corrupted envelope (`not an envelope`), unknown survey (`survey not found`), malformed definition.
- **Worker fetch failure:** the first module build `50d2c187…90dd` failed every run with `wasm_url could not be fetched`, even though FastFS served it with a matching hash. A fresh 190 KB spike and a 341 KB padded spike, uploaded the same way minutes later, both ran, so it isn't size or the CLI. A rebuilt module (new hash) ran fine. Most likely a fetch failure got cached against that hash. Mitigation: if a new version won't fetch, rebuild to a new hash, or host the wasm at another HTTPS URL (see OutLayer #32 and #33).

## Full cycle on `discovery-dev` (2026-11-05)

- Survey `3b474ea3…b852`: created on-chain, two responses via `discovery submit` (one with identifying `context_only` details: clinic, city, merchant account, email, tx ids, volumes), finalized on-chain after the deadline with secrets row `nearai`.
- `Finalize` released a report in ~44 s (`z-ai/glm-5.3-flash`). It contains none of the planted identifiers; "healthcare" appears only as the approved phrasing. It kept the publishable repro steps as a **reproducible** finding, kept the disagreement, and proposed two acceptance-criteria changes.
- After `Finalize`, `status` shows no survey key, so stored ciphertext can't be decrypted. Responses are deleted (covered by tests; can't be listed from outside by design).
- Bugs found and fixed:
  - `wasi-http-client` writes the body in one `blocking-write-and-flush`, which traps above 4096 bytes. Replaced with a chunked client (`crates/module/src/http.rs`).
  - GLM-5.3 Flash reasons by default (budget 8192) and reasoning counts toward `max_tokens`, so `max_tokens: 4096` gave no content. Now `thinking_token_budget: 1024` for synthesis, `reasoning_effort: low` for the audit.
  - **Error outputs are public on-chain.** NEAR AI errors and schema-parse errors could echo model text. Errors now carry only status, `finish_reason` and token counts; a schema mismatch is a retried attempt, never an error.
  - `status` failed after `Finalize` (key deleted). It now returns the definition, closed state, the key while open, and the report once released.
  - The report referred to a respondent label ("R2"). The prompt now forbids labels and attributing views to individuals.
- FastFS: two uploads were never indexed (404 after 10+ minutes). Re-uploading worked. Upload scripts now poll with a deadline and retry.

## CLI: on-chain key verification and operator commands (2026-11-05)

- `discovery verify` / `submit` check the survey key from the `InitSurvey` transaction:
  - service signer, OutLayer contract, single `request_execution`;
  - `execution_requested` for the pinned project and release hash, with an `init_survey` input whose definition hashes to the survey id;
  - `execution_completed` success, and the output key.
  
  Then, **at the init block** (archival RPC), the release must be the only version and active, and the owner must have no access keys.
- On the dev project, strict mode refuses ("3 versions at init … owner still had 1 access key"). `--allow-unlocked` passes with warnings. A wrong release pin is refused.
- `discovery operator create|finalize` sign NEAR transactions themselves (borsh FunctionCall, ed25519 from the NEAR CLI keychain). They submit with `broadcast_tx_async` and poll `tx`, because a 45–90 s OutLayer run outlasts the RPC's `send_tx` wait (HTTP 408).
- **Withheld report, diagnosed:** a dev-only `diagnostics` build showed the mechanical checks passed. The model audit rejected disclosures the respondents had published themselves, because it never saw the authorized material. Fixed to ask D3's question. My first hypothesis (6-word rule false positives) was wrong.
- **Fix confirmed:** survey `bd3d9849…5b67` (diagnostics build `0b708621…1d8c`, same synthetic responses, `withheld_terms: ["Dayton"]`) was released on the first attempt, with no rejections recorded. None of the planted identifiers appear and there are no respondent labels. Full CLI path: `operator create` → `submit` ×2 → `operator finalize` → `report`.
- Note: `discovery-dev` currently runs a diagnostics build. That's fine for dev. A real release must be built without the feature.

## Review findings, fixed with regression tests (2026-11-05)

- **A. Plaintext leaked in `Submit` errors.** After decryption, `serde_json` errors echo values. Reproduced: ``json: unknown variant `SECRET-PLAINTEXT-7731` ``. These errors leave the enclave through OutLayer's front end. Now every post-decryption rejection is a fixed message. Unreadable stored records in `Finalize`, whose errors are public on-chain, give a generic error.
- **B. Retention gap.** If a `Finalize` run stored the report and died before discarding, later calls returned early and never deleted responses or the survey secret. Discarding is now one idempotent step that runs on both paths.
- **C. Replay could undo an edit.** OutLayer's front end sees every envelope, and resubmitting an old one overwrote a newer edit. Responses now carry a signed `sent_at_ms`. A submission replaces the stored one only if strictly newer; stale or replayed envelopes get the same "accepted" reply but aren't stored, so the replayer learns nothing.
- **E. Markdown abuse.** Model text went into GitHub Markdown unescaped (`@mentions` notify people, raw HTML). `Report::markdown` now escapes `@`, `<`, `>`, `&`.
- Dev build `975e5629…0219` (diagnostics) deployed; smoke test passed: create, verified submit, fixed error output.

Known limits, not fixed (disclosed or out of MVP scope):
- "Discard" is logical deletion in OutLayer's storage, whose backups and retention aren't documented. Any retained ciphertext is sealed under the project key and can only be opened by the locked module, which never outputs plaintext.
- Respondents trust their NEAR RPC provider for verification data.
- `Finalize` puts every response in one prompt. Very large surveys will hit time and context limits.

## First locked release (2026-11-05)

- Release account `rel1.research.agency.testnet`; project `rel1.research.agency.testnet/discovery`, one version: release build `74bd94d7990829a9f67796aeb808c6c26434f9c823614ccc94a83b1abcd5c3fa` (no `diagnostics`).
- NEAR AI secret row owned by `research.agency.testnet`, accessor `Project(rel1…/discovery)`, access `whitelist:research.agency.testnet and build:74bd94d7…c3fa`.
- Owner key `ed25519:ENrxjgBy…qX6D` deleted. The account is locked (no access keys).
- Strict cycle, no development flags: survey `e815da5f…89c9` (init tx `Fa8Ta8Yk…k3Tu`); `discovery verify` → `verified: true` with no warnings; two strict submits; `operator finalize` (tx `GNANnBGG…hybf`) → **released**. None of the planted identifiers, no labels, no diagnostics. Status: closed, key deleted, report released.

## Attested, end-to-end encrypted Finalize with receipts (2026-11-05)

- Ported from `MultiAgency/private-ai@dd469a5` into `crates/nearai`, with files kept byte-identical and a provenance header: `attest`, `e2ee`, `sign`, `receipt`, `net`. The module's `Confidential` client (`crates/module/src/confidential.rs`) does three things before any response leaves the enclave:
  - verifies a fresh NEAR AI attestation report: Intel TDX quotes for the gateway and model via `dcap-qvl` with Phala PCCS collateral, plus NVIDIA's signed GPU verdict, bound to our nonce;
  - encrypts every request to the attested model key (E2EE v2), so the gateway relays only ciphertext;
  - uses a reply only after the model enclave's ed25519 signature over the exact request and response bytes checks out.
- Their HTTP client replaced ours. It starts the request before writing the body, which fixes a potential stall on bodies larger than the stream buffer.
- The audit now fails closed: output without a valid verdict is a rejection (test `audit_without_verdict_withholds`).
- Dev build `af05f563…74bc` (diagnostics, 1.21 MB), survey `8ca336e9…2c6d`: **released** on the first attempt in one run, with none of the planted identifiers. Receipt: 355 KB, receipt v2 format (attestation, NVIDIA verdict and key, nonce, two signed turns with request/response hashes, OutLayer run info), `sha256` = the report's `receipt_sha256` (`6d88c17b…`). It contains no survey content. The only probe-word hit was "payment" in NEAR AI's gateway compose file.
- Network allowlist: `cloud-api.near.ai` (120 s), `pccs.phala.network`, `nras.attestation.nvidia.com`.
