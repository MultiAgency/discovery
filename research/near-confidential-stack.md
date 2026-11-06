# NEAR AI confidential inference + OutLayer: feasibility for a confidential agent-survey MVP

Researched 2026-11-04. Sources are primary only: docs.near.ai, NEAR AI's live API, near.ai policies, OutLayer docs (app.outlayer.ai), and OutLayer source (github.com/out-layer/outlayer, shallow clone at commit `276c654`, 2026-11-04). The reference repo `near-research/survey-tool` was read from a local clone and nothing in it was run.

**Legend.** **[V]** means verified in docs, source, or a live API response, with the URL given. **[I]** means my inference, not stated by any source. Unmarked sentences summarise the [V] claims around them.

---

## TL;DR

- **NEAR AI Cloud** is an OpenAI-compatible API with real, client-verifiable attestation: Intel TDX quotes, NVIDIA GPU evidence through NRAS, per-response signatures, and optional E2EE to an attested model key. The guarantees only cover models tagged "Confidential TEE". "Incognito" models (Anthropic, OpenAI, Gemini) leave the TEE. NEAR AI publishes no explicit prompt/log retention period.
- **OutLayer** runs hash-pinned WASI (wasm32-wasip2) modules inside Intel TDX. It provides TEE-only secrets (`PROTECTED_*`, generated in the TEE), keystore-encrypted persistent storage, outbound HTTPS, and per-run attestations that bind wasm hash, input, output, caller, and project. It has **no built-in scheduler**. Its HTTPS front door (the coordinator) is **closed source and sees request bodies**, and the **project owner can push a new WASM version that inherits all secrets and storage**.
- The full chain is buildable in a hackathon. The client encrypts to a TEE-derived key whose public key is attested by a run of a specific wasm hash. Ciphertext goes to OutLayer worker storage. An external trigger at the deadline starts a run that decrypts, calls NEAR AI (ideally E2EE), filters, and posts to GitHub with a TEE-held token.
- **The honest claim:** organizers, OutLayer's host and operator, NEAR AI, and GitHub cannot see individual responses, *provided the deployed build is the audited one*. **The claim we cannot make:** that the survey service operator (whoever owns the OutLayer project) is technically unable to see responses. Hardware does not prevent a code swap. The swap is only *detectable* after the fact, unless the project is made immutable (see §4).

---

## 1. NEAR AI Cloud: confidential inference

### 1.1 What runs where
- **[V]** Hardware is Intel TDX confidential VMs plus NVIDIA GPU TEE. Private LLM nodes have 8x NVIDIA H200 GPUs, TDX-enabled CPUs, and a "Private-ML-SDK" for attestation and signing. https://docs.near.ai/cloud/private-inference.md
- **[V]** The gateway `cloud-api.near.ai` "runs in its own TEE" and forwards to model TEEs over HTTPS. It handles routing, API keys, and usage tracking for billing. https://docs.near.ai/cloud/private-inference.md
- **[V]** NEAR AI states that infrastructure providers, model providers, and NEAR "cannot access your data at any point", and that responses can be signed inside the TEE. https://docs.near.ai/cloud/private-inference.md
- **[V]** The catalog has three privacy tiers. https://docs.near.ai/cloud/models.md
  - **Confidential TEE** runs on NEAR AI's own fleet with attestation, signatures, and verification.
  - **3P Confidential TEE** runs in a third-party enclave (e.g. Chutes) with the provider's own evidence. The JS SDK verifies Chutes model attestation; the Python SDK verifies only the gateway.
  - **Incognito** (OpenAI/Anthropic/Gemini) is proxied through the TEE gateway, but the model runs outside a TEE, "so the TEE privacy and verifiability guarantees do not extend to the upstream provider."
- **[V]** The live model list shows attested models, with USD per 1M tokens (input/output). Source: https://cloud-api.near.ai/v1/model/list, fetched 2026-11-04. Pricing comes from the same endpoint.

  | Model | Provider type | Price in / out | Notes |
  | - | - | - | - |
  | `z-ai/glm-5.3-flash` | `vllm` (NEAR fleet) | $0.08 / $0.50 | 1M context; tools, structured outputs |
  | `Qwen/Qwen3.6-35B-A3B-FP8` | `vllm` | $0.17 / $1.10 | |
  | `Qwen/Qwen3.8-27B` | `vllm` | $0.04 / $1.35 | |
  | `deepseek-ai/DeepSeek-V4.1-Flash-1017` | `vllm` | $0.30 / $1.20 | |
  | `openai/privacy-filter` | `vllm` | $0.01 / – | PII-redaction model |
  | `z-ai/glm-5.3` | `tinfoil` | | 3P TEE |
  | `moonshotai/kimi-k3` | `chutes` | | 3P TEE |

  **[I]** `providerType: vllm` probably corresponds to the "Confidential TEE" tier, and `chutes`/`tinfoil` to "3P". The docs don't state this mapping.
- **[V]** Stale-doc trap. The quickstart uses `zai-org/GLM-5.1-FP8` (https://docs.near.ai/cloud/quickstart.md). The live list shows it is now an *alias* of `z-ai/glm-5.3-flash`. Aliased requests get a `gateway` signature instead of a `provider_tee` signature (https://docs.near.ai/cloud/verification/cloud-api/response-signatures.md). **Always send the canonical model ID with `x-no-aliasing: true`.**

### 1.2 API shape, auth, SDKs
- **[V]** The API is an OpenAI-compatible REST API at `https://cloud-api.near.ai/v1`. Auth is `Authorization: Bearer sk-...` (an API key from the dashboard). Other token types are JWT access, `rt_` refresh, and `rpt-` reporting. https://docs.near.ai/api-reference/introduction.md
- **[V]** Gateway responses carry an opaque `X-Request-Id`. https://docs.near.ai/cloud/quickstart.md
- **[V]** `/v1/responses` is stateless and accepts only `store:false`. NEAR AI "does not store the raw request, response, output items, or conversation history for later retrieval." https://docs.near.ai/cloud/guides/stateless-responses.md
- **[V]** The SDK packages are `@nearai/inference-sdk` (Node ≥24.22.1), `nearai-inference-sdk` (Python ≥3.12), Rust `nearai-inference-sdk`, and Go. They verify gateway attestation before sending, verify model attestation "where supported", and offer `verifyResponse()`, optional E2EE, and optional OHTTP. Response records expire after 60 minutes by default. https://docs.near.ai/cloud/inference-sdk/index.md
- **[V]** The Rust SDK examples assume Tokio. https://docs.near.ai/cloud/inference-sdk/rust.md **[I]** This means the SDK is unlikely to run inside an OutLayer wasm32-wasip2 module as-is. Plan to hand-roll the HTTP call and E2EE.
- **[V]** `POST /v1/privacy/redact` uses `openai/privacy-filter` to replace detected PII with realistic dummies. `/v1/privacy/classify` returns PII spans. https://docs.near.ai/api-reference/privacy/post-v1privacyredact.md, https://docs.near.ai/api-reference/privacy/post-v1privacyclassify.md
- **[V]** A 429 means `rate_limit_exceeded` or `service_overloaded` (same pages). Per-org concurrent-request limits are set by admins. https://docs.near.ai/llms.txt (admin endpoints "get/update organization concurrent request limit")
- **[V]** Billing works by prepaid credits (https://docs.near.ai/cloud/quickstart.md) or "Staking for Inference", where staking rewards become $1 credits (https://docs.near.ai/cloud/staking-for-inference/overview.md).
- **[V]** Direct per-model endpoints `{slug}.completions.near.ai` are **experimental**. Their attestation covers one instance, and signature lookups can 404. They are not recommended. https://docs.near.ai/cloud/experimental/direct-completions.md

### 1.3 Attestation flow (how a client verifies)
- **[V]** Fetch evidence with `GET /v1/attestation/report?nonce=<64-hex>&signing_algo=ecdsa|ed25519[&model=<id>&provider=near][&include_tls_fingerprint=true]`. An API key is required, and the call is free. The response contains `gateway_attestation` and, when requested, `model_attestations[]`. https://docs.near.ai/cloud/verification/cloud-api/gateway-attestation.md, https://docs.near.ai/cloud/guides/e2ee-chat-completions.md
- **[V]** Verification steps, per https://docs.near.ai/cloud/verification/reference/quote-nonce-signer.md:
  1. Verify `intel_quote` with Intel DCAP (`dcap-qvl`) and apply your TCB-status policy.
  2. Reject TDX debug mode.
  3. Check that `report_data` equals the 32-byte signer identity (or `SHA256(identity‖tls_spki_fp)` when TLS binding is used) followed by the 32-byte client nonce.
  4. Replay the RTMR3 event log.
  5. Check that `MRCONFIGID == 0x01‖SHA256(app_compose)‖0^15`.
  6. Send `nvidia_payload` to NVIDIA NRAS and require `x-nvidia-overall-att-result == true`.
  7. Apply your own measurement and image-provenance allowlist.
- **[V]** Response integrity: `GET /v1/signature/{chat_id}`. A `provider_tee` signature covers `MODEL:sha256(req):sha256(resp)` from the model TEE. A `gateway` signature covers only the gateway's relayed bytes. https://docs.near.ai/cloud/verification/cloud-api/response-signatures.md
- **[V]** Documented caveats from NEAR AI:
  - "Instances of the same model share one signing key", so a matching signer does not prove that the same instance served the request. https://docs.near.ai/cloud/verification/cloud-api/model-attestations.md
  - Gateway plus model evidence plus a signature "do not yet form a complete model-to-Gateway-to-final-response chain." https://docs.near.ai/cloud/verification/cloud-api/response-signatures.md
  - TLS binding requires the peer cert and the report on the *same* TLS connection, which browsers cannot do. https://docs.near.ai/cloud/verification/cloud-api/tls.md
  - Attestation doesn't establish image provenance without a separate check. https://docs.near.ai/cloud/verification/reference/verification-policy.md
- **[V]** E2EE: encrypt message fields to the model's attested `signing_public_key` using X25519 + HKDF-SHA256 + XChaCha20-Poly1305 (v2). Required headers are `X-Signing-Algo: ed25519`, `X-Client-Pub-Key`, `X-Model-Pub-Key`, `X-Encryption-Version: 2`, and `x-no-aliasing: true`. Responses are encrypted to the client key. E2EE works on chat/completions/embeddings/images but **not** on `/v1/responses`. https://docs.near.ai/cloud/guides/e2ee-chat-completions.md
- **[I]** With E2EE, the gateway (and anything in front of it) sees only ciphertext plus metadata. This is the strongest available posture, and it doesn't need TLS binding.

### 1.4 What the operator can and cannot see
- **Can see [V]:** request metadata for routing, billing, and usage (https://docs.near.ai/cloud/private-inference.md); org-level usage, cost, and request-ID data (https://docs.near.ai/llms.txt, usage and reporting endpoints).
- **Cannot see (claimed, attestable) [V]:** prompts and outputs on Confidential-TEE models. This is enforced by TDX/GPU-TEE isolation and is checkable through the flow in §1.3.
- **Logging/retention [V]:** the privacy policy (last updated 12 Oct 2026) gives no inference-specific retention period. It says data is kept "only for as long as needed to provide and secure the Services, or as required to comply with law," allows de-identified data to be used "for any purpose", and allows disclosure to authorities. https://near.ai/privacy-policy
- **[I]** The privacy of Confidential-TEE prompts rests on the TEE, not on the policy. Don't cite the policy as a retention guarantee.

---

## 2. NEAR OutLayer

### 2.1 What it is and its status
- **[V]** OutLayer offers "verifiable compute and custody for AI agents". You build a WASI program and publish the `.wasm` named by its sha256. OutLayer runs it in Intel TDX and returns an enclave-signed result. It is called either from a NEAR contract (`request_execution` on `outlayer.near` / `outlayer.testnet`, using yield/resume) or over HTTPS (`POST https://api.outlayer.ai/call/{owner}/{project}`). https://app.outlayer.ai/llms.txt
- **[V]** Workers run in dstack TDX CVMs, either on Phala Cloud or on self-hosted bare metal. They register on `worker.outlayer.near`, which checks the Intel DCAP quote and all 5 measurements (MRTD + RTMR0–3) against an allowlist. https://app.outlayer.ai/docs/trust-verification
- **[V]** That measurement allowlist is owner-controlled (`add_approved_measurements` → `assert_owner()`). https://github.com/out-layer/outlayer/blob/main/register-contract/src/lib.rs
- **[V]** Production apps exist (e.g. near.email). The contracts, worker, keystore, and SDK are open source (MIT). **The coordinator (HTTPS API, task queue, billing) is closed source.** https://github.com/out-layer/outlayer/blob/main/README.md, https://github.com/out-layer/outlayer/blob/main/API.md
- **Stale or moved [V]:**
  - The repo moved from `fastnear/near-outlayer` to `out-layer/outlayer` (301 redirect). The docs moved from `outlayer.fastnear.com` to `app.outlayer.ai` (301).
  - The NEAR docs page https://docs.near.org/primitives/off-chain-compute/outlayer.md still says OutLayer "clones, compiles, and caches" GitHub code. The current docs say versions are prebuilt, hash-pinned wasm, and that secret-holding workers do no compilation (https://app.outlayer.ai/docs/getting-started, README "No Compilation in TEE").
  - The reference repo uses `outlayer = "0.1"`. The docs show `0.2`, and say SDK 0.1 binaries "keep running". https://app.outlayer.ai/docs/storage

### 2.2 Deployment model
- **[V]** Build with `cargo build --target wasm32-wasip2`. `outlayer upload` publishes to FastFS and prints the sha256. `outlayer deploy` creates a project version. The worker refuses bytes that don't match the hash. https://app.outlayer.ai/docs/getting-started
- **[V]** Only WASI P2 supports HTTP and storage. WASI P1 has neither. https://app.outlayer.ai/docs/wasi
- **[V]** A project is `{owner}/{name}` with versions and an active version. **All versions share storage and project secrets.** Only the owner can add versions or switch the active one. https://app.outlayer.ai/docs/projects, `add_version`/`set_active_version` in https://github.com/out-layer/outlayer/blob/main/contract/src/projects.rs
- **[V]** An HTTPS caller can pin a version with `version_key` (the wasm hash). https://app.outlayer.ai/docs/https-api
- **[V]** Resource limits: default 1B instructions / 128 MB / 60 s; maximum 500B instructions / 512 MB / 180 s. https://app.outlayer.ai/docs/https-api, https://app.outlayer.ai/docs/pricing
- **[V]** Each outbound HTTP request defaults to a 30 s timeout. https://github.com/out-layer/outlayer/blob/main/worker/src/connector_manifest.rs (`DEFAULT_HTTP_TIMEOUT_SECS`)
- **[V]** Pricing is `base_fee + instructions×rate + time×rate`. Unused budget is refunded; failed runs are not. HTTPS calls are paid in USD stablecoin via payment keys (`X-Payment-Key: owner:nonce:secret`). On-chain calls are paid in NEAR. https://app.outlayer.ai/docs/pricing, https://app.outlayer.ai/docs/https-api

### 2.3 Secrets
- **[V]** Secrets are encrypted in the browser or CLI to the keystore's public key (ECIES X25519/HKDF/ChaCha20-Poly1305) and stored on-chain. They are decrypted only in the TEE and injected as env vars. https://app.outlayer.ai/docs/secrets
- **[V]** `PROTECTED_*` secrets are generated inside the TEE ("nobody ever sees the value"), and manual secrets may not use that prefix. However, "at runtime, the WASM code does have access to the decrypted value … a malicious developer could potentially exfiltrate it through a backdoor in their code." https://app.outlayer.ai/docs/secrets
- **[V]** Secrets can be bound to a project (all versions) or to a WASM hash. The "One build only" rule admits a single wasm sha256, but the secret owner can move it to a new build without re-entering the value. Access conditions include whitelist, balance, regex, expiry, and AND/OR/NOT. https://app.outlayer.ai/docs/secrets
- **[V]** CLI: `outlayer secrets set` overwrites; `update` merges and preserves `PROTECTED_*`; `delete` removes the profile; `--generate PROTECTED_X:hex32|ed25519|…` creates TEE-generated values. https://github.com/out-layer/outlayer/blob/main/docs/CLI.md (inlined in https://app.outlayer.ai/llms-full.txt)
- **[V]** The keystore master comes from NEAR MPC Confidential Key Derivation, gated by DAO approval (`dao.outlayer.near`) of the keystore's TDX attestation. It is deterministic and survives restarts. Per-project keys are derived with HMAC-SHA256. https://app.outlayer.ai/docs/secrets, https://app.outlayer.ai/docs/trust-verification
- **[V]** "Author secrets" (manifest `author_secrets`) decrypt the author's credential into every run, and their access condition gates who may run the project at all. https://app.outlayer.ai/docs/secrets

### 2.4 Storage
- **[V]** `set`/`get` are encrypted by the keystore TEE with a key derived from `storage:{project_uuid}:{account_id}`. Each caller's cell is isolated, and code can read a user's cell only when that user triggers the run. https://app.outlayer.ai/docs/storage
- **[V]** `set_worker`/`get_worker` (`@worker`) is project-wide shared state, encrypted by default. `is_encrypted=false` makes it **public**, readable over `GET /public/storage/get`. https://app.outlayer.ai/docs/storage
- **[V]** Raw records are readable by the operator ("The operator can read raw records"). Atomic ops include `set-if-absent` and CAS. Deleting a project deletes its storage. https://app.outlayer.ai/docs/storage
- **[I]** Survey responses must go to `@worker` storage, because a deadline run triggered by someone else cannot read respondents' own cells.

### 2.5 Keys derived in the TEE (signing / encryption)
- **[V]** Signing keys (ed25519/secp256k1) and encryption keys (symmetric) are derived per project uuid (or per exact wasm) **and per caller**. The module never sees them. https://app.outlayer.ai/docs/signing-keys, https://app.outlayer.ai/docs/encryption-keys
- **[I]** Because these keys are per caller and the API is sign/seal only (no ECDH export), they **don't fit** a single survey-wide decryption key. A `PROTECTED_*` secret is the practical way to get one, as both the private-DAO example and the reference repo do.

### 2.6 Attestation
- **[V]** Every run's TDX quote commits (via a "task hash" in `report_data`) to input, output, wasm hash, caller, payment, and project. https://app.outlayer.ai/docs/trust-verification, https://app.outlayer.ai/docs/signing-keys
- **[V]** HTTPS responses include `attestation_url` (`/attestations/by-call/{call_id}`). Only hashes of HTTPS payloads are stored, so the caller must keep the request and response. https://app.outlayer.ai/docs/https-api, https://app.outlayer.ai/docs/trust-verification
- **[V]** `outlayer-verify` (Apache-2.0) checks attestations offline. https://github.com/out-layer/outlayer-verify
- **[V]** OutLayer's own stated limits: a passing verdict "does not prove the approved build came from the published source" (you need reproducible builds), and it doesn't tell you who may approve builds. https://app.outlayer.ai/docs/trust-verification
- **[V]** The TEE does not guarantee code correctness or prevent side channels. https://app.outlayer.ai/docs/tee-attestation

### 2.7 Networking, scheduling, inference
- **[V]** WASI P2 modules can make outbound HTTPS calls. The AI example calls `api.openai.com` with a key stored as a secret. https://app.outlayer.ai/docs/wasi, https://app.outlayer.ai/docs/examples
- **[V]** The egress allowlist (`capabilities.network` or `network` in the wasm's `outlayer.manifest` custom section) is **mandatory for connectors and opt-in for every other project**. With no allowlist, egress is unrestricted. When declared, the worker enforces it inside the TEE, and because it lives in the wasm, it is covered by the hash. The source's own example lists `cloud-api.near.ai`. https://github.com/out-layer/outlayer/blob/main/worker/src/connector_manifest.rs
- **[V]** No scheduler is documented. The separate `out-layer/self-hosted-scheduler` (v0.1.0, early, self-hosted, untrusted) triggers HTTPS calls on interval, storage-diff, or webhook, with no cron or one-shot time. https://github.com/out-layer/self-hosted-scheduler
- **[V]** Block timestamp and height are exposed only in on-chain mode (`NEAR_BLOCK_TIMESTAMP`). In HTTPS mode they are empty strings. https://github.com/out-layer/outlayer/blob/main/wasi-examples/WASM_ENV_VARS.md
- **[V]** There is no first-party NEAR AI integration in the docs. Calling NEAR AI is just an outbound HTTPS call. **[I]**
- **[V]** A curated GitHub connector exists, with writes like `issue_comment` that can be configured to require owner confirmation. https://app.outlayer.ai/docs/tasks, https://app.outlayer.ai/docs/connectors

### 2.8 Closest example
- **[V]** `private-dao-example` is close to our design:
  1. The TEE generates a master secret.
  2. The TEE derives a public key.
  3. The frontend encrypts votes to it, and the votes are stored on-chain.
  4. A finalize run decrypts, tallies, and returns only aggregates plus Merkle inclusion proofs.
  5. Dummy votes hide participation.

  https://app.outlayer.ai/docs/examples, https://github.com/out-layer/private-dao-example

---

## 3. The end-to-end chain: link by link

"Attested" means a third party can check it cryptographically. "Trusted" means it relies on someone's code or behaviour that attestation doesn't pin.

| # | Link | How | Attested or trusted |
| - | - | - | - |
| 1 | The client gets the survey public key | Run `GetPublicKey`. The module derives a secp256k1/X25519 public key from `PROTECTED_MASTER_KEY` (plus a survey-ID tweak) and outputs it. The client checks that run's attestation: Intel quote, approved worker measurements, task hash binding output (the public key) to `wasm_hash` and `project_id`. | **Attested [V]**, per "Prove a key is the project's" at https://app.outlayer.ai/docs/signing-keys. The client must also check that `wasm_hash` equals the audited reproducible build. **Trusted:** the source-to-hash mapping, unless a reproducible build is published. |
| 2 | The client encrypts the response | Done locally (ECIES). This is needed because **the coordinator sees HTTPS bodies** and **on-chain `input_data` is public**. | Client-side; no trust needed beyond the client's own code. **[I]** The coordinator reads inputs: it parses `operation` out of the input to bill (https://app.outlayer.ai/docs/connectors), and it is closed source (https://github.com/out-layer/outlayer/blob/main/API.md). |
| 3 | The ciphertext is stored | A `Submit` run writes the ciphertext to `@worker` encrypted storage, keyed by a hash of the respondent's identity for dedup. This double-wraps it: ECIES plus keystore encryption. | **Partly attested.** Storage encryption is done by the attested keystore [V]. Durability and availability are **trusted**: the coordinator DB holds it, and the operator could delete it. **[I]** The operator can't read it, but can withhold it. |
| 4 | The deadline trigger | Something external calls `Finalize`: a GitHub Actions cron, the self-hosted scheduler, or anyone. The module must enforce its own rules: deadline passed, minimum N respondents, **one-shot** (a `set-if-absent("finalized")` flag), and no submissions accepted after finalization. | **The trigger itself is trusted for liveness only.** In-module rules are **attested** (they are part of the hashed code). Time is a weak point: in HTTPS mode there's no block timestamp [V], so it relies on the host clock **[I]**. Use on-chain mode for `NEAR_BLOCK_TIMESTAMP`, or treat early triggering as harmless *given* one-shot plus a minimum-N rule. |
| 5 | The TEE decrypts | `Finalize` reads `@worker` records and decrypts with `PROTECTED_MASTER_KEY`. | **Attested**, but only while the same build runs. **Trusted:** the project owner not shipping a version that dumps keys or plaintext. All versions inherit secrets and storage [V]. |
| 6 | Confidential inference | The module calls `cloud-api.near.ai` with an API key from a secret, preferably E2EE to a model key it verified from `/v1/attestation/report`, using the canonical model ID and `x-no-aliasing`. | **Attested if the module verifies.** In-WASM DCAP verification plus a NRAS call is feasible but heavy **[I]**. Without it, you're trusting standard TLS to the NEAR AI gateway plus NEAR AI's claims. Whether WASI's HTTP client exposes the peer certificate for TLS binding is unknown. **[I]** E2EE sidesteps that. |
| 7 | Publication filter | In-module code: k-anonymity threshold, drop quotes, run `/v1/privacy/redact` on candidate output, enforce length and structure limits, block verbatim n-gram overlap with any single response. | **Attested as code.** The LLM's *behaviour* isn't verified: prompt injection by respondents is possible **[I]**. Deterministic post-filters are the part you can actually defend. |
| 8 | Post to GitHub | The module calls `api.github.com` with a fine-grained token (issues:write on one repo) stored as a project secret, locked with "One build only" and an egress allowlist of `cloud-api.near.ai` and `api.github.com`. Alternative: the curated GitHub connector. | **The token is TEE-held [V]**, but readable by any build its owner approves [V]. The egress allowlist is **attested** and stops the module sending data elsewhere [V], *if declared*. GitHub sees only the report. |
| 9 | Audit trail | Publish the `call_id`, attestation URL, wasm hash, and model signature in the GitHub comment. | **Attested [V]**, checkable with `outlayer-verify`. |

---

## 4. Reference repo `near-research/survey-tool` (NEAR Forms) and the coordinator's questions

### What the reference repo does (from code)
- It is a Rust wasm32-wasip2 module using `outlayer = "0.1"` with actions `SubmitForm`, `ReadResponses`, and `GetMasterPublicKey`. (`/tmp/ref/survey-tool/wasi-near-forms-ark/src/main.rs`)
- `SubmitForm` validates the EC01 format, never decrypts, and POSTs the ciphertext **plus `submitter_id` in plaintext** to an external db-api/Postgres. (`main.rs` `handle_submit_form`; `db-api/migrations/20260226000001_forms_schema.sql` stores `submitter_id TEXT NOT NULL`)
- `ReadResponses` lets the **creator decrypt every individual response**, once `signer_account_id == FORM_CREATOR_ID` is checked. Output is encrypted to an ephemeral client key so it isn't public on-chain. (`main.rs` `handle_read_responses`, README)
- **This is the opposite of our requirement.** We would remove `ReadResponses` and replace it with a one-shot `Finalize`.
- The client gets the public key from a static `NEXT_PUBLIC_MASTER_PUBLIC_KEY` env var (README). Nothing attests it, so whoever controls the web UI can substitute a key. A `GetMasterPublicKey` action exists but isn't attestation-checked by the client.
- Calls use transaction mode (`request_execution` via the wallet), so **the respondent's account appears on-chain as signer of a call to that project**. (`web-ui/src/lib/near.ts`)

### (a) Can anyone read a dashboard-generated `PROTECTED_MASTER_KEY`? Who can rotate or replace it?
- **[V]** It is generated inside the keystore TEE, and the developer never sees it. https://app.outlayer.ai/docs/secrets
- **[V]** It is decrypted into the env of *every run the access condition admits*. With a project binding, that means **any version the project owner deploys**, including one that prints the key. https://app.outlayer.ai/docs/secrets, https://app.outlayer.ai/docs/projects
- So OutLayer's operator and host cannot read it [V]. The project owner can, indirectly, by deploying exfiltrating code. A "One build only" lock moves the decision to the *secret owner*, who can repoint the lock [V]. If the secret owner and project owner are the same account, the lock doesn't protect against that account.
- **Rotate or replace [V]:** the secret owner can `delete`, or `set` (overwrite), the profile and generate a fresh `PROTECTED_*`. Manual values can't impersonate the `PROTECTED_` prefix. **[I]** Replacing the key makes old ciphertext undecryptable, so it's a DoS, not a disclosure.
- **[V]** At the infrastructure level, the keystore master depends on DAO approval of keystore builds and on NEAR MPC, and worker measurements depend on the owner of `worker.outlayer.near`. A malicious approved build there could read all secrets. That's visible on-chain, but it's a trust root.

### (b) Can a client attest that a public key comes from a specific wasm or version? Can the owner silently swap the WASM?
- **Attest the key: yes [V].** Have the module output the public key and verify that run's attestation. The task hash binds output, `wasm_hash`, and `project_id`. https://app.outlayer.ai/docs/signing-keys
- Clients can also pin `version_key` on HTTPS calls [V] (https://app.outlayer.ai/docs/https-api). But the *finalize* caller picks its own version.
- **Swap: yes, not silently but without consent [V].** `add_version` and `set_active_version` are owner-only contract calls (public on-chain), and the new version inherits project secrets and storage. https://github.com/out-layer/outlayer/blob/main/contract/src/projects.rs, https://app.outlayer.ai/docs/projects
- The `Finalize` attestation names the `wasm_hash` that ran, so a swap is detectable in the audit trail [V].
- **[I] Hardening options:**
  1. Own the project and the secrets from an account whose full-access keys are deleted after deployment (a locked account). No one can then add versions or move locks. The cost is that bugs can't be fixed.
  2. Own it from a multisig or DAO.
  3. Use wasm-hash binding (`WasmUrl` source plus hash-bound secrets), which is immutable by construction. But storage is per wasm hash without a project, and project storage requires a project (https://app.outlayer.ai/docs/storage), so this pushes us toward on-chain or external storage.

### (c) Is there a non-transaction mode for agent CLIs? Does it give signer identity?
- **[V]** Yes: `POST https://api.outlayer.ai/call/{owner}/{project}` with `X-Payment-Key`, or a trial `Authorization: Bearer wk_...` from `POST /register` with no auth. https://github.com/out-layer/outlayer/blob/main/API.md
- **[V]** Identity is `NEAR_SENDER_ID` / `signer_account_id()`, which equals the **payment key's owner**. https://app.outlayer.ai/docs/web2-integration, https://app.outlayer.ai/docs/signing-keys
- **[I]** Each respondent agent therefore needs its own NEAR account plus payment key (or a `wk_` custody wallet) for per-respondent identity. If one shared service key is used, identity must be proven inside the encrypted payload instead, e.g. a signature from the respondent's key.

### (d) Can modules make outbound HTTPS to arbitrary hosts? What are the limits?
- **[V]** Yes, from WASI P2. Egress is unrestricted unless the manifest declares an allowlist (opt-in for non-connectors). https://github.com/out-layer/outlayer/blob/main/worker/src/connector_manifest.rs
- **[V]** Limits: ≤180 s wall clock, ≤512 MB, ≤500B instructions, 30 s default per request (≤180 s per host entry). https://app.outlayer.ai/docs/https-api
- **[V]** There is no documented egress byte limit. **[I]**

### (e) Is there a scheduled, cron, or deadline trigger?
- **[V]** Not in the platform. The external `self-hosted-scheduler` offers interval, storage-diff, and webhook triggers only. https://github.com/out-layer/self-hosted-scheduler
- So something external must call `Finalize`. A GitHub Actions `schedule` is the simplest. **[I]**

### (f) Is transaction-mode I/O, including the signer, public on-chain?
- **[V]** "Input and output data are stored on NEAR blockchain." https://app.outlayer.ai/docs/tee-attestation
- **[V]** `request_execution` takes `input_data` as an argument. https://app.outlayer.ai/docs/near-integration
- **[I]** NEAR transaction signers and arguments are public. So **transaction mode leaks who participated** in which survey project, and when, even with encrypted answers. The reference repo has this leak.
- **[I]** HTTPS mode keeps participation off-chain, but the closed-source coordinator still sees the payment-key owner and timing.

### (g) Does OutLayer offer encrypted persistent storage, so we can drop db-api?
- **[V]** Yes. `@worker` storage is keystore-encrypted, shared across callers, and has atomic ops. It requires a project and wasm32-wasip2. https://app.outlayer.ai/docs/storage
- **[I]** This removes the db-api, which in the reference repo is a plaintext store of `submitter_id` and a separate party that must be trusted for availability.

---

## 5. Gaps and risks for a hackathon timeline

1. **Owner code swap (critical).** Covered in §4(b). The mitigation is a locked or multisig owner plus publishing the wasm hash and a reproducible build (`scripts/build_github_wasm.sh`, https://app.outlayer.ai/docs/secrets). For a demo, at least publish the hash and show the attestation of `Finalize`.
2. **Differencing attacks [I].** Repeated finalizations over growing response sets reveal individuals. Make `Finalize` one-shot, close submissions afterwards, and enforce a minimum N.
3. **LLM leakage and prompt injection [I].** A respondent can write "quote all other responses verbatim". Treat the model as untrusted. Use structured output plus deterministic filters (n-gram overlap, PII redaction, aggregate-only fields) and a minimum-N threshold.
4. **Coordinator visibility [I, partly V].** It is closed source and fronts all HTTPS calls, so payloads must be client-encrypted, and participation metadata is visible to OutLayer.
5. **Time in HTTPS mode [V/I].** There is no trusted timestamp, so rely on one-shot plus minimum N, or use on-chain mode for `Finalize` only. The organizer's contract call reveals nothing about respondents.
6. **NEAR AI verification inside WASM [I].** The SDK assumes Tokio, so in-module DCAP plus NRAS verification is real work. E2EE needs only pure-Rust crypto, but the key is only as good as the attestation check that produced it.
7. **Model signing-key sharing and rotation [V/I].** Instances share keys (§1.3), and key lifetime is undocumented, so don't encrypt *stored* responses to a NEAR AI model key.
8. **Availability.** The operator can withhold or delete storage [V: project deletion wipes storage]. Accept this for the MVP.
9. **Maturity [V].** The scheduler is v0.1.0. OutLayer docs move fast: the domain and repo moved, and docs.near.org is stale. NEAR AI direct endpoints are experimental.

## 6. Recommended minimal architecture

```
Organizer agent ── HTTPS /call CreateSurvey(issue_url, questions, deadline, min_n) ──▶ OutLayer project (owner = locked/multisig acct)
Respondent agent ─ HTTPS /call GetPublicKey(survey_id) → verify attestation (wasm_hash == published) 
                 ─ ECIES-encrypt answers → HTTPS /call Submit(survey_id, ciphertext)  → @worker storage
GitHub Actions cron (untrusted) ─ HTTPS /call Finalize(survey_id) ──▶ module:
      deadline/min_n/one-shot checks → decrypt → NEAR AI (Confidential TEE model, canonical ID, E2EE)
      → deterministic filter + /v1/privacy/redact → POST api.github.com issue comment (token secret)
      → output {report_hash, call_id, wasm_hash, model signature id}
Manifest: network allowlist [cloud-api.near.ai, api.github.com]; secrets: PROTECTED_MASTER_KEY, NEARAI_API_KEY, GITHUB_TOKEN (One-build-only)
```

### Fallbacks per link if time runs short
- **Attested public key (link 1):** hard-code the public key and wasm hash in the client, verify the attestation once by hand, and say so in the README.
- **Locked owner (§4b):** keep a normal owner. Publish the wasm hash and, in the report, the `Finalize` attestation that shows which hash ran. The claim becomes "detectable, not preventable".
- **In-WASM NEAR AI verification (link 6):** call over plain TLS with a canonical Confidential-TEE model ID. Log the `chat_id` and verify the `provider_tee` signature out-of-band afterwards with the SDK. Note that the signature is retrievable only for a limited window (records expire after 60 min by default, per https://docs.near.ai/cloud/inference-sdk/index.md, possibly SDK-specific).
- **E2EE (link 6):** skip it and rely on the gateway TEE plus TLS.
- **OutLayer storage (link 3):** store the ciphertext in a GitHub gist or repo, or any DB. It is already ECIES-encrypted, so the store is untrusted. The cost is that dedup and closing logic move outside the TEE.
- **Deadline trigger (link 4):** the organizer runs `Finalize` manually. The one-shot plus minimum-N rule keeps this safe.
- **GitHub post from the TEE (link 8):** `Finalize` returns the report. Any party (CI) posts it, and readers verify `report_hash` against the attested output. The token never needs to enter the TEE.
- **If OutLayer is not workable at all [I]:** use NEAR AI alone, with respondents E2EE-encrypting to a model key and an orchestrator batching ciphertexts into one chat request. **Rejected as a main path:** whoever holds the API key controls the prompt and can ask the model to echo every response. Nothing attests the orchestrator's code, and model-key lifetime is undocumented.

---

## Unknowns (the docs don't answer these)
- Whether the OutLayer coordinator logs or retains HTTPS request bodies, and for how long. It is closed source, and no retention statement was found.
- Whether a WASI P2 module can see the TLS peer certificate, which NEAR AI TLS binding needs, or run `dcap-qvl` within the limits.
- The lifetime and rotation policy of NEAR AI model E2EE and signing keys.
- NEAR AI's prompt/metadata log retention for Cloud inference, and default per-org rate limits.
- Whether NEAR AI attestation is *mandatory* for `provider_tee` signing on every Confidential-TEE model, or only "where supported".
- Whether the WASI clock inside OutLayer workers can be trusted (the host controls the TDX guest's time source).
- Who controls `worker.outlayer.near`'s owner account and `dao.outlayer.near` membership, and under what process measurement approvals happen.
- OutLayer egress or bandwidth caps beyond time limits; and whether `@worker` storage has size limits or a cost per byte for large surveys.
- Whether a project's ownership can be made immutable in a supported way, beyond deleting the owner account's access keys **[I]**.
