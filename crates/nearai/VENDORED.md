# Source provenance and local patches

Base: `MultiAgency/private-ai@dd469a5`, `app/src/{attest,e2ee,sign,receipt,net}.rs`.
This crate is derived code, not a byte-identical vendor snapshot.

Local security changes (November 5, 2026):

- `attest.rs`: verify every returned model candidate before inference and retain
  the verified set. Reject duplicate signer identities, invalid candidates, and
  recorded GPU-verdict arrays whose length differs from the candidate set.
  E2EE selects the first verified model; the signed-turn checker still requires
  exactly that selected key. All accepted candidates have distinct signer keys,
  making that match unique within the retained evidence.
- `net.rs`: retain every candidate's GPU verdict and verification key.
- `receipt.rs`: extend version-2 receipts with `model_gpu_tokens` and
  `model_gpu_keys`, ordered like the preflight candidate array. Keep the original
  selected `gpu_token`/`gpu_key` for compatibility. A legacy single verdict is
  sufficient only for a single-candidate report; multi-candidate evidence must
  supply the full verdict array. The unused version-1 offline verifier and
  upstream-app claim-rendering helpers have been removed; the product CLI
  verifies OutLayer's on-chain report output, not the inference receipt.

Policy source:
https://docs.near.ai/cloud/verification/cloud-api/model-attestations

Tests use an injected hardware-quote verifier and actual ES384 signatures on
synthetic NVIDIA-format JWTs. They check candidate policy, not Intel hardware,
NVIDIA's production signing keys, or a deployed inference service.

Scope of assurance: a uniquely matched signing key is not proof of a unique
serving instance; NEAR AI documents shared keys across instances. Applying an
approved workload-measurement/image-provenance policy is a separate deployment
review. No new measurement allowlist has been invented by these patches.

MVP scope alignment:
- `e2ee.rs`: reject weak Ed25519 keys and non-contributory X25519 exchanges.
  Support text-only requests; unsupported content shapes fail rather than pass
  through in plaintext. Remove unused tool-call encryption/decryption.
- `net.rs`: signed turns accept text messages only, with no tool definitions.
- All Rust sources use the workspace's standard rustfmt formatting.
