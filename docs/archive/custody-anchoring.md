> Historical material. The current scope is [the November 4 concept](../concept.md); these documents do not define current features or operating instructions.

# Optional custody-wallet anchoring

Default submissions continue to use unlinkable per-survey local keys. Anchoring
is explicit: it links a stable local key to a custody wallet, inside encrypted
submissions. OutLayer sees the one-time signing setup; it is not told which
surveys the anchor is later used for. The enclave can link anchored responses
across surveys, as disclosed in D6.

## Setup once

Use a private identity directory and keep its `respondent-secret`. Local identity
commands need no survey, project, organizer or release arguments.

```bash
umask 077
export DISCOVERY_HOME=<private-identity-directory>
discovery identity anchor-request > anchor-request.json
```

The request contains only the stable local public-key anchor message and
recipient `discovery`; it includes no survey ID, definition, answer or timestamp.
After owner approval, sign it with the owner's existing OutLayer wallet, using
the matching network host. For example, with a wallet credential already stored
securely in the environment:

```bash
curl --fail --silent --show-error \
  -H "Authorization: Bearer $OUTLAYER_WALLET_API_KEY" \
  -H 'Content-Type: application/json' \
  --data-binary @anchor-request.json \
  https://testnet-api.outlayer.ai/wallet/v1/sign-message > wallet-signature.json

discovery identity import-anchor wallet-signature.json
```

The CLI verifies the wallet's implicit account and NEP-413 signature over the
expected local key before saving `custody-anchor.json` with private permissions.
A signature for another local key or a different existing anchor is refused.
No wallet credential is imported or stored by this flow. Do not post either
setup file on GitHub or announce participation.

## Submit

With the survey's verified release settings:

```bash
discovery submit <survey> --init-tx <init_tx> --anchored answers.json
```

Use the same identity directory and `--anchored` for edits. This verifies the
saved anchor locally, signs with the stable key, then encrypts the entire
submission through the shared service payment key. There is no wallet API call
at submission time. Missing or mismatched identity files cause an error; the CLI
does not switch identities automatically. Keep one identity mode per survey:
switching between anchored and unanchored mode creates a different respondent
slot rather than editing the original response.

Reference:
https://app.outlayer.ai/docs/agent-custody (Sign Message / NEP-413).
