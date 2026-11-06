# Documentation check for prototype simplification

Reviewed November 5, 2026. Requirements baseline: `docs/concept.md`.
This is a documentation and source comparison, not a fresh deployment test.

## Sources and conclusions

1. Entry skill and application-building guidance:
   - https://skills.outlayer.ai/outlayer/SKILL.md
   - https://skills.outlayer.ai/building-outlayer-apps/SKILL.md
   - https://skills.outlayer.ai/building-outlayer-apps/rules/critical-rules.md

   WASI P2 is appropriate for this module's outbound inference and key-host
   interfaces. The critical rules distinguish application output from platform
   transport: on-chain JSON output is the module's value; HTTPS has its own
   status/output envelope. Keep these parsers separate. The installed SDK's
   `env::output_json` calls its output helper, which flushes stdout.

2. Encryption keys and storage:
   - https://skills.outlayer.ai/building-outlayer-apps/references/encryption-keys.md
   - https://skills.outlayer.ai/building-outlayer-apps/references/wasi-tutorial.md
   - Installed, pinned `outlayer` 0.2.1: `src/storage/sealed.rs`,
     `src/storage.rs`, `src/encryption_keys.rs`.

   Project-bound keys span versions; build-bound keys cannot be used through a
   project. Keep the one-project-per-locked-release design. Key paths and caller
   identity must stay stable. Raw records need sealing and sensitive names need
   MACs. Our response record names contain MACs; definition/report prefixes
   contain public survey hashes.

   The SDK provides sealed records and ciphertext compare-and-swap. It is not a
   drop-in replacement for our `Records`: it MACs the complete name, uses the
   logical name as AAD, and returns opaque tags when listing. Our layout uses
   survey prefixes to enumerate responses for finalization and deletion. A
   direct replacement would change storage addressing and require an index;
   assess that total complexity before adopting it.

   Follow-up correctness work: submission edits currently compare timestamps
   and then unconditionally write. Interleaved runs could overwrite a newer
   version. Use a bounded CAS loop over the ciphertext that was read, and test
   interleavings. Also test submission/finalization interleavings and a crash
   between receipt and report writes. These are source-level risks, not live
   concurrency reproductions performed in this pass.

3. HTTPS execution context:
   - https://app.outlayer.ai/docs/https-api

   HTTPS uses the payment-key owner as sender and has no block timestamp.
   This supports the shared-service-account design and on-chain-only
   finalization. Keep explicit version pinning. Distinguish transport failure
   from execution failure; a blind resend can duplicate a charged run.

4. OutLayer execution provenance and project ownership:
   - https://app.outlayer.ai/docs/tee-attestation
   - https://raw.githubusercontent.com/out-layer/outlayer/main/contract/src/projects.rs

   The contract registers attested worker authority to resolve executions.
   Project source restricts version additions and changes to the owner.
   Our report verification relies on the configured NEAR RPC and OutLayer
   contract; it is not an independent Intel quote verification path. Published
   main-branch source is supporting evidence, not proof of deployed bytecode.

5. NEAR AI verification:
   - https://docs.near.ai/cloud/verification
   - https://docs.near.ai/cloud/verification/cloud-api/gateway-attestation
   - https://docs.near.ai/cloud/verification/cloud-api/model-attestations
   - https://docs.near.ai/cloud/private-inference

   Fresh evidence must be verified before confidential content is sent. Lazy
   initialization retains this order. Current model guidance requires checking
   every returned candidate and uniquely matching the later signer against
   retained verified evidence. It also warns that model instances share signing
   keys, so signer equality alone does not establish serving-instance identity.
   Our vendored verifier returns on the first valid candidate and does not
   implement that full policy. Update it deliberately with fixtures and receipt
   implications before treating the inference verification review as complete.
   Preserve E2EE, nonce/measurement binding, GPU evidence and signed-turn checks.

## Artifact checks

The locally built release component contains the `outlayer.manifest` marker and
imports `near:storage/api@0.2.0`,
`outlayer:encryption-keys/api@0.1.0`, and WASI HTTP interfaces. Checked with
`wasm-tools component wit` plus inspection of the artifact bytes. This confirms
packaging, not successful execution on a deployed worker.

## Scope

The three implemented simplifications remain structurally appropriate:
remove the CLI's inference dependency, initialize inference only when needed,
and prepare batch data once. No additional production code or deployment was
changed during this documentation check. Some linked environment/worker source
URLs could not be retrieved; the HTTPS environment table and pinned SDK were
used where available. The original concept remains unchanged.

## Follow-up implementation status

The source-level response-edit race and first-valid-candidate policy mismatch
were reproduced with deterministic offline tests and patched. Receipt/report
mismatch after a simulated crash was also reproduced and patched with
content-addressed receipts. See the final section of `docs/decisions.md` and
`crates/nearai/VENDORED.md`. Survey-wide closure interleavings, workload/image
allowlist policy and deployed end-to-end verification remain separate work.
