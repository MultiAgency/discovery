> Historical testnet experiment. The custody adapter has been removed from
> the supported organizer workflow in favor of local signing through the
> official NEAR CLI. See [current operations](../docs/operations.md).

# Custody organizer verification

Verified on testnet, November 5, 2026. This exercises the actual Rust custody
adapter against the existing synthetic platform probe, not the survey module
or a confidential model.

| Item | Evidence |
| --- | --- |
| Service wallet / organizer | `ce145b9bd4768450856549ccc62a31d01a3a491d0187a0208cdbc8723f496a84` |
| Probe project | `spike1.research.agency.testnet/spike` |
| Probe build | `8a6ef8e2c704979141130c5608753d571734cc03ca742d4bb434bfb37ebe61f8` |
| Custody execution | [ARSGt26iSospysk93sd9K5Q7kudF3SjSeW6i6nAQ37LC](https://explorer.testnet.near.org/transactions/ARSGt26iSospysk93sd9K5Q7kudF3SjSeW6i6nAQ37LC) |
| Bootstrap funding | [0.5 testnet NEAR](https://explorer.testnet.near.org/transactions/F9GwaEs7Zoxb7iGQJGkATHpQGxhoTFa4UayUcmLz3j5s) from the existing spike account |

The probe confirmed:

- The custody executor and public key match the organizer's implicit account.
- The broadcast transaction has the expected signer, receiver, sole function
  call, arguments, gas and deposit. Survey output comes from NEAR RPC, not the
  custody API's result object.
- The OutLayer execution events name that organizer, project and pinned build.
  On-chain mode exposes the trusted block timestamp.
- Sending the same operation key again returns the same transaction rather
  than creating a second execution.
- A funded submission key owned by that wallet, restricted to the spike
  project with a $0.02 per-call cap, runs HTTPS under the same caller.
- HTTPS derives the same MAC and opens the sealed record written on-chain.
  It does not insert a new record into a different caller namespace.

The key was funded with one fake testnet USDC. Wallet and submission credentials
remain in ignored local files, with newly created key files restricted to mode
0600; none are included here. No mainnet transaction or model call was made.

## API compatibility finding

The live address endpoint returns its public key as `ed25519:<64 hex digits>`;
the written examples describe base58. The adapter accepts either encoding only
when the decoded 32 bytes equal the pinned organizer account. Offline tests
cover both formats and a mismatched key. A read-only check reproduced the
encoding difference before any probe transaction was submitted.

This experiment did not activate a survey release or migrate any existing
survey. The funded testnet wallet and its credentials were not deleted or
repurposed when changing the signing implementation.
