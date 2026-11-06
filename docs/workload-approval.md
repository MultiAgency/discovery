# Workload approval for the next release

`crates/module/workload-policy.json` is included in the module binary. It is
intentionally empty until a workload review exists. The module refuses both
survey creation and confidential inference without a nonempty policy. This
prevents an immutable release from collecting responses it cannot finalize.
There is no organizer-supplied or secret-profile override.

The gate checks the canonical model ID and the SHA-256 of the raw attested
compose configuration for every verified model candidate. It is evaluated after
quote/GPU checks and before any response content is transmitted to the model.
The gateway remains a ciphertext relay under E2EE; its hardware evidence is
still verified. Model compose approval is additional to those checks.

## Required review evidence

Before adding a record:

1. Obtain fresh model evidence from the gateway with the canonical model ID,
   `provider=near`, a client nonce, and no aliasing. Verify every candidate.
2. Verify the quoted configuration measurement against the raw compose bytes.
   Inspect every image reference; require immutable image digests.
3. Verify source/build provenance for required images: exact subject digest,
   trusted repository, workflow, build identity and source revision. A registry
   response, image tag or observed compose hash is not approval.
4. Record the accepted configuration hash and a public review/evidence reference.
   Include all approved configurations that may appear as model candidates.
5. Build and review a new module, then publish it as a new locked project. The
   binary hash covers this policy. Keep existing release registry entries fixed.

Record shape (placeholders are deliberately invalid):

```json
{
  "model": "z-ai/glm-5.3-flash",
  "approved_workloads": [
    {
      "compose_sha256": "<reviewed 64-character configuration hash>",
      "approval_reference": "<source/build provenance review reference>"
    }
  ]
}
```

The runtime implements configuration pinning, not a Sigstore/provenance verifier.
The review must establish why the pinned configurations are trusted. No approved
production hash was established during the local implementation work; none has
been guessed or copied from an unverified deployment.

Sources:
- https://docs.near.ai/cloud/verification/cloud-api/model-attestations
- https://docs.near.ai/cloud/verification/cloud-api/image-provenance
