# OutLayer docs review against D1–D5 and proposed D6

Reviewed 2026-11-05.

**Sources and a caveat about them.** `/tmp/ol.txt` (llms-full.txt) does **not** contain the dashboard docs pages. It inlines only repository markdown: README, API.md, AUTHENTICATION.md, CLI.md, WASI tutorials, CUSTODY.md, VAULTS.md, LEAVING_OUTLAYER.md, VRF.md and WORKER_ATTESTATION.md (see `/tmp/ol.txt:19-41`). The pages these questions depend on (signing-keys, encryption-keys, storage, account-binding, agent-connect, binding-api, tasks, connectors, secrets, projects, https-api, payment-keys, vaults, trust-verification, tee-attestation, examples) were fetched as server-rendered HTML from `https://app.outlayer.ai/docs/<page>` and converted to text in `/tmp/olpages/*.txt`. Source citations are to `/tmp/ref2/outlayer` at commit `276c654`. Three live **public, unauthenticated** API GETs were made to settle Q3; they are marked **[V-live]**.

**Legend.** **[V]** means verified: a URL or file:line plus a quote. **[I]** means my inference.

---

## TL;DR

- **D6's shape is right, and no native mechanism beats it.** Every native identity path goes through OutLayer's coordinator, and through it to a **public job log**: `NEAR_SENDER_ID`, Personal Account Binding / `use_bound_identity`, per-caller signing and encryption keys, per-caller storage cells, and custody `sign-message`. A shared payment key plus an in-ciphertext NEP-413 signature is the only documented way to keep respondent identity away from the coordinator and the public.
- **D6 needs four fixes before it is decidable:**
  1. **`@worker` storage has no `set-if-absent`, no CAS and no `list-keys`.** Only `set-worker`/`get-worker` exist, so one-response-per-account dedup cannot be atomic there and Finalize cannot enumerate records. Under a shared payment key the **caller's cell is one shared cell** (the service account's), and it has `set-if-absent-raw`, `list-keys` and CAS. Use that cell, with raw records keyed by `mac()`.
  2. **Pin the survey key with `author_secrets`, not `secrets_ref`.** Any account can store a project-bound secret for *our* project under its own name. A caller-named `secrets_ref` therefore lets a stranger make our attested wasm run with *their* `PROTECTED_*` key.
  3. **Encrypted storage leaks an unsalted SHA-256 of every key name.** A name like `survey:42:alice.near` can be confirmed by the storage operator. HMAC/`mac` names are required, not optional.
  4. **The Submit output must not be guessable.** The public attestation record carries `output_hash`, so constant or random-salted outputs only.
- **Participation hiding is weaker than assumed.** `GET https://api.outlayer.ai/public/jobs` (no auth) lists every execution with `user_account_id`, `project_id`, `call_id`, `created_at`, `execution_time_ms` and `instructions_used` [V-live]. With a shared key it still publishes **how many Submit runs happened and when**, to everyone, the organizer included. The coordinator also sees respondent IPs.
- **The RPC weak spot can be shrunk, not removed.** The worker has an undocumented host interface `near:rpc` with `view-access-key` / `view-access-key-list`. The query then leaves from OutLayer's TEE over TLS to OutLayer's RPC provider (FastNEAR), not from anything we run. It is capped at 100 calls per execution. Nothing in the docs or source gives key lookups without some RPC provider seeing the account.
- **Better alternative for posting (D4-compatible):** drop the GitHub token from the TEE. Finalize writes the report to **public `@worker` storage** (`is_encrypted=false`, readable at `/public/storage/get`) and returns it as attested output. The maintainer's agent posts it. A TEE-held GitHub token would also need renewing, which a locked secret owner (D5) cannot do.
- **D5:** no documented "renounce" mechanism exists. A locked owner blocks `add_version`, `transfer_project` and `delete_project`. But **every registered version stays callable by anyone via `version_key`**, so remove all but the final version before locking. **MPC Vaults undermine D1** (the parent can unilaterally exit and recover the master) and break under a locked parent. Do not use them. Nothing found requires the project or secret owner to sign during normal operation.

---

## Q1. Identity

### What each native mechanism gives the module, and to whom it reveals the caller

- **HTTPS signer identity = payment-key owner.** [V] https://app.outlayer.ai/docs/https-api: "`NEAR_SENDER_ID` | Transaction signer | Payment Key owner". [V] Source: `worker/src/main.rs:1455-1464`: "NEAR_SENDER_ID — the guest-visible identity: the verified bound asset account when the wallet operates one (Agent Connect), the payment key owner otherwise. NEAR_USER_ACCOUNT_ID always stays the payment key owner". With one shared service key, the module sees the service account for every respondent. That is useless for identity, which is why D6 has to carry identity inside the ciphertext.
- **Signing keys (signer vs predecessor).** [V] https://app.outlayer.ai/docs/signing-keys: "caller | signer (default) | NEAR_USER_ACCOUNT_ID | … | Over HTTPS: the payment key's owner", and for predecessor: "Over HTTPS: the signer". [V] Same page: "Give each caller a stable ed25519 identity per project — a NEAR implicit account that can sign NEP-413 messages." Per-caller derivation keys on the *payment-key owner*. A shared key gives one key for all respondents, and per-respondent keys put the respondent in the public log (Q3). **No help for D6.** [I]
- **Agent Custody (`wk_`, agent IDs, NEP-413).** [V] https://app.outlayer.ai/docs/agent-custody: "Every wallet is identified by its Agent ID — a public key derived deterministically inside the TEE", the NEAR address being a "Derived implicit account". NEP-413 signing is a coordinator endpoint: "`POST /wallet/v1/sign-message`", "message | Yes | Text to sign (max 10,000 bytes)". [I] Using it means the coordinator receives the signed message and knows which wallet signed. The wallet is implicit, so it also fails D6's named-only gate. **Worse than D6.**
- **Custody trial keys cannot call our project.** [V] https://app.outlayer.ai/docs/payment-keys: "Calls on a trial key | 10 in all | In the wallet's first week, connectors only". [V] https://app.outlayer.ai/docs/connectors: "A trial is fifty connector calls, within seven days". The docs disagree on 10 vs 50, but both say connectors only. Sponsor codes also cover only connectors: "The subscription pays for connector calls."
- **Personal Account Binding / Agent Connect.** This is the only native way to give the module a **named** account, and the chain check runs in the enclave. [V] https://app.outlayer.ai/docs/binding-api: "Set use_bound_identity and the guest runs as the bound account… The worker re-verifies it against the chain inside the enclave before setting the guest's environment". But:
  - It requires the respondent's own wallet payment key: "X-Payment-Key: <a key the wallet owns>".
  - It requires a one-time install on the respondent's account. [V] https://app.outlayer.ai/docs/account-binding: "You install a small wallet contract on your own account" and "Use an account with no contract on it." It also needs a NEP-591-capable wallet.
  - It leaves a payer record: "the earnings ledger records the payer rather than the borrowed identity."
  - [I] The payer's wallet account appears in the public job log (Q3), and the extension set links it on-chain to the named account. **Publicly reveals participation, so it is incompatible with D1/D2.**
  - Agent Connect is "Mainnet is not open yet" and is for partner-leased accounts. Not applicable.
- **Verdict:** no native mechanism proves respondent identity to the module without revealing it to the coordinator and the public. D6's in-ciphertext NEP-413 signature is the correct shape. [I]

### Avoiding the NEAR RPC lookup

- **Host RPC inside the worker exists, but the docs under-document it.** [V] https://app.outlayer.ai/docs/wasi: "Host functions … give your code privileged access to private NEAR RPC endpoints (powered by Fastnear)". That page lists only `view`/`call`/`transfer`. [V] Source `worker/wit/world.wit:20,24` also exports `view-access-key: func(account-id, public-key, finality-or-block)` and `view-access-key-list: func(account-id, finality-or-block)`. They are proxied by the worker (`worker/src/outlayer_rpc/host_functions_sync.rs:451-490`) and hard-capped at 100 calls per execution (`worker/src/executor/wasi_p2.rs:446`: "100, // max_calls"). The worker logs only the method name, not the params: `host_functions_sync.rs:130` `info!("[RPC] Sending {} request to {}", method, …)`.
  - [I] This moves the lookup from "our module calls some RPC over wasi-http" to "OutLayer's TEE calls OutLayer's RPC provider with OutLayer's API key". The respondent's IP is never linked, and the query is mixed into all OutLayer traffic. It does **not** hide the account from the RPC provider. FastNEAR is plausibly the same organisation as OutLayer (the repo moved from `fastnear/near-outlayer`), so the RPC provider may be the coordinator operator. Neither is in D1's threat set (organizer, us), but disclose it.
- **No-RPC verification exists only for implicit accounts.** [V] https://app.outlayer.ai/docs/agent-custody: "For implicit accounts, the verifier can confirm identity without RPC: account_id == hex(public_key_bytes)." D6's named-only gate is exactly what forces the lookup. [I] Named accounts are also weak Sybil resistance, because anyone with one account can create sub-accounts cheaply. That is NEAR general knowledge, not from these docs.
- **A design option that decouples lookup timing (inference).** At Submit, dedup on `mac(survey_id ‖ account ‖ pubkey)` with no lookup. At Finalize (or a pre-finalize pass), verify `pubkey ∈ full-access keys(account)` via host `view-access-key`, optionally with decoy lookups, and keep one record per account. A forged record under someone else's name has a key that won't verify, so it can't squat the real holder's slot. Cost: at most about 100 lookups (real plus decoy) per run.
- **Require FullAccess keys.** [V] OutLayer's own NEAR-signature auth does: https://app.outlayer.ai/docs/agent-custody: "The key must be a full-access key of the account … a function-call key, the kind applications hold after you log in to them, is refused." `view_access_key` returns the permission, so check it. [I]
- **Fix the NEP-413 `recipient` in the module** (e.g. `survey.<project>`), and include `survey_id`. [V] https://app.outlayer.ai/docs/signing-keys: "A caller-chosen NEP-413 message and recipient is a login." [I] The same reasoning applies on the verify side, so signatures can't be replayed from another app.

---

## Q2. Dedup and storage

- **`@worker` has no atomic ops and no listing.** [V] `worker/wit/deps/storage.wit:103-120` shows that only `set-worker` and `get-worker` operate on `@worker`. [V] https://app.outlayer.ai/docs/storage: "Every storage function outside the -worker ones reads and writes one account's cell of the project". So `set-if-absent`, `set-if-equals`, the `-raw` functions and `list-keys` act **only on the caller's cell**.
  - [I] D6's "store in @worker under HMAC for dedup" cannot be made race-free: 10 concurrent jobs are allowed per key (https://app.outlayer.ai/docs/payment-keys: "Concurrent jobs per key | 10"). Finalize also cannot enumerate `@worker` without a hand-maintained index, itself racy.
- **Under a shared key, the caller cell *is* a single shared cell.** [V] https://app.outlayer.ai/docs/storage: "storage_account … "signer" (default) | … Over HTTPS: the payment key's owner". Every respondent submitting with the service key writes the service account's cell. Finalize must be called with a payment key of the **same owner account** (any nonce) to read it. [I] This is also a feature: a stranger calling Finalize with their own key sees an empty cell. Risk: if the service account ever has to change, the records are stranded in the old cell and the `mac` keys change too (see next bullet).
- **The `mac` host function can replace our own HMAC secret.** [V] https://app.outlayer.ai/docs/encryption-keys: "mac(path, vault, data) | ok: HMAC-SHA256 of data, 32 bytes, under a subkey derived from the key for this purpose alone … Deterministic". The page also says "Name records by a keyed hash (mac) so that their names are hidden too". The limit is "at most 262144 bytes (256 KiB) … of mac data".
  - Caveat: the key is per (project uuid, caller). [V] Same page: "The keystore derives it inside the TEE from its master, for this project (or this exact build) and this caller." [I] Under the shared key that is stable. It changes if the service account changes.
  - Upside over a `PROTECTED_*` HMAC secret: the module never holds the key ("the host encrypts, decrypts or authenticates and hands back the result, never the key").
- **Encryption keys are symmetric only.** They can't serve as the respondent-facing survey key: `encrypt`/`decrypt`/`mac` only, with no public half [V same page]. So the survey key stays a `PROTECTED_*` secret, which confirms the prior research's §2.5 inference.
- **Are storage KEYS visible?**
  - **Raw records:** name and bytes are in the clear. [V] https://app.outlayer.ai/docs/storage: "The operator can read raw records. The key name and the value are stored as given." [V] https://app.outlayer.ai/docs/encryption-keys: "storage keys are visible to the operator".
  - **Encrypted records (`set`, `set-worker`):** the name is encrypted, **but an unsalted SHA-256 of it is stored next to it.** [V] `keystore-worker/src/api_support.rs:1617-1618`: "// Calculate key hash for unique constraint … let key_hash = hex::encode(Sha256::digest(req.key.as_bytes()));". [V] `worker/src/outlayer_storage/mod.rs:32`: "Keys are hashed (SHA256) for the unique constraint". [I] The storage operator can confirm any guessed name, e.g. `dedup:42:alice.near`.
  - Key names also appear in debug logs: the worker logs `storage::set_worker key={}` (`worker/src/outlayer_storage/host_functions.rs:98`), and the keystore logs `key = %req.key` (`keystore-worker/src/api_support.rs:1597`). Both are debug level, and whether those logs leave the CVM is unknown.
  - **Conclusion:** use `mac` names for every record. [I]
- **What "raw storage" and "the operator" mean.** Raw storage means the `-raw` functions store "bytes as given … and make no keystore call" [V storage page]. "The operator" is whoever runs the coordinator's PostgreSQL, i.e. OutLayer, **not** the project owner. [V] `worker/src/outlayer_storage/mod.rs:1-23`: "Records are stored in the coordinator's PostgreSQL database." [I] We as project owner have no direct read path. Storage is reachable only through runs of the project.
- **Recommended storage recipe [I]:**
  - Key: `hex(mac("dedup", none, survey_id ‖ account))`.
  - Value: the respondent's ECIES ciphertext, which is already sealed to the survey key. Pad it to a fixed size, because ciphertext length leaks response length.
  - Write: `set-if-absent-raw`, after signature verification. Verifying first means the inserted/duplicate answer is not an oracle for "has X responded".
  - Finalize: `list-keys` over the cell.
  - Raw records also avoid a bottleneck: `list-keys` on *encrypted* records decrypts every key through the keystore under a 25 s budget. [V] `worker/src/outlayer_storage/client.rs:437`: "const LIST_KEYS_TIMEOUT … from_secs(25)", and lines 446-449: "list_keys timed out … too many keys".
- **Size limits and costs:** not documented for storage. I found no per-value cap in the worker client [I]. The coordinator is closed source. Known ceilings: 256 KiB per `mac`/`encrypt` call [V], 180 s per run (https://app.outlayer.ai/docs/https-api), 100 RPC calls per run [V source], and the 25 s `list-keys` budget [V source]. Deleting the project deletes storage [V storage page].

---

## Q3. Participation hiding

- **The private-DAO example hides whether a member voted, not who the members are.** [V] https://app.outlayer.ai/docs/examples: "DAO members can send encrypted noise instead of real votes to hide voting activity from observers" and "Dummy messages: Send encrypted noise to hide whether you voted (indistinguishable from real votes on-chain)". Votes are `contract.cast_vote(...)` transactions, so the signer is public. Its deploy steps also generate the master **locally**: "openssl rand -hex 32 > dao_master_secret.txt" … "Secrets JSON: {"DAO_MASTER_SECRET":"<paste hex from file>"}". It is WASI P1. [I] The model for us is only "dummies make real submissions indistinguishable"; its key handling is not.
- **There is a public, unauthenticated execution log listing callers.**
  - [V] `/tmp/ol.txt:577` (AUTHENTICATION.md): "`GET /public/executions` - List execution history" under "Public Endpoints (No Auth Required)".
  - [V] https://app.outlayer.ai/docs/tee-attestation: "Go to /executions to see the history of all code executions on OutLayer."
  - [V] https://app.outlayer.ai/docs/trust-verification: "Anyone can open any execution".
  - **[V-live]** `GET https://api.outlayer.ai/public/jobs?limit=1` returned `"user_account_id":"ref-labs.near"`, `"project_id":"price-oracle.near/price-oracle"`, `"is_https_call":true`, `"call_id":…`, `"created_at":…`, `"execution_time_ms"`, `"instructions_used"` and `"payment_key_nonce":1`. It is pageable with `offset`.
- **Per-run attestations are public and name the caller.**
  - **[V-live]** `GET https://api.outlayer.ai/attestations/221092` returned `caller_account_id`, `payment_key_owner`, `payment_key_nonce`, `input_hash`, `output_hash`, `project_id`, `secrets_ref` and `timestamp`.
  - [V] `worker/src/tdx_attestation.rs:150-152`: the task hash covers "caller_account_id || project_id || secrets_ref || timestamp || attached_usd". The docs page https://app.outlayer.ai/docs/tee-attestation gives an older algorithm without these V1 fields, so it is stale.
- **Payment-key usage is public.** [V] `/tmp/ol.txt` API.md table: "`/public/payment-keys/{owner}/{nonce}/balance` · `/usage` | Payment key balance / usage". **[V-live]** `/public/payment-keys/ref-labs.near/1/balance` returned `spent`, `available` and `"last_used_at"`. The coordinator also keeps history. [V] https://app.outlayer.ai/docs/payment-keys: "Off-Chain (Coordinator) … Usage history (audit log)".
- **IPs are visible to the coordinator.** [V] payment-keys: "Requests per minute (IP) | 100 | Before key validation". Whether bodies or IPs are retained is unknown.
- **Project-owner dashboard.** Earnings shows "Payment History | Individual payments with timestamps, caller info, and amounts" ([V] https://app.outlayer.ai/docs/earnings). With a shared key and no attached deposit, that is only the service account. [I]
- **Consequences [I]:**
  - Per-respondent payment keys or bound identities would publish participation to **anyone**, the organizer included. The shared key is therefore **mandatory**, not just nice to have.
  - Even with the shared key, the public log shows the **count and timestamps** of runs on the survey project, plus `execution_time_ms`/`instructions_used`. Those can distinguish GetPublicKey, Submit-accepted and Submit-rejected unless padded.
  - Mitigations:
    - route every operation through the same code path with similar cost;
    - have respondent agents add random delay;
    - optionally send dummy submissions (the private-DAO pattern), which a cheap "noise" flag inside the ciphertext enables;
    - make Submit output constant, or random-salted, so `output_hash` can't be brute-forced to "accepted for alice.near";
    - pad ciphertexts.
  - The organizer learning n is consistent with D2; learning *timing* is a new, disclosable leak.

---

## Q4. GitHub

- **A curated GitHub connector exists. It acts *as the GitHub login's owner*, called by an agent with its own payment key.** [V] https://app.outlayer.ai/docs/connectors: "The agent makes one HTTPS call naming the operation, paid with its own payment key". [V] `connectors/github-connector/README.md`: "An agent works in the owner's GitHub account — **as the owner**", and every post carries a marker (default "— posted by an AI agent via OutLayer").
- **"Every GitHub write asks first" means every write *can* wait for approval, and the default is none.** [V] https://app.outlayer.ai/docs/tasks: "A connector can hold a write until its owner approves it. The owner chooses which writes, and the default is none". The table lists "GitHub | every write: … issue_comment …". The "owner" here is whoever connected the GitHub login, approving in the OutLayer inbox ("approves with one message their wallet signs"). It is **not** the OutLayer project owner, so there is no direct conflict with D5. [I]
- **Fit [I]:**
  - Calling the connector *from inside* our Finalize module would need an agent payment key in our secrets and egress to `api.outlayer.ai`. That pattern is undocumented.
  - A TEE-held GitHub token (author secret) is worse under D5. Fine-grained PATs and app tokens expire, and a locked secret owner cannot renew them. Alternatively the maintainer stores the token as their own project-bound secret, named via `secrets_ref`, which any account can do ([V] `contract/src/secrets.rs:2004-2011` checks only that the project exists).
- **Simpler, D4-aligned alternative [I]:**
  - Finalize writes the report to public worker storage: [V] https://app.outlayer.ai/docs/storage: "`set_worker_with_options(…, Some(false))` // <-- Makes it public!", readable at `GET https://api.outlayer.ai/public/storage/get?project=…&key=…`.
  - Finalize also returns the report as output, attested by `output_hash`. The default worker output cap is 10 MB: `worker/src/executor/mod.rs:229` "DEFAULT_MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024".
  - The maintainer's agent posts it, through the GitHub connector or its own token, with the attestation link. Anyone can check that the posted text matches the attested output.
  - This removes a secret, an egress host and a renewal dependency. Privacy is unaffected, because whoever posts holds only the already-public report.

---

## Q5. D5: locked owner

- **No renounce or immutability primitive.** [V] `contract/src/projects.rs` public methods: `create_project`, `add_version`, `set_active_version`, `remove_version`, `delete_project`, `accept_project_transfer`, `revoke_project_transfer`, `transfer_project`, plus views. There is no freeze or renounce. Transfer exists. [V] `projects.rs:515` `transfer_project` requires a prior `accept_project_transfer` by the new owner and "Only project owner can transfer project". [V] https://app.outlayer.ai/docs/signing-keys: "it survives a transfer of the project: the id changes, the uuid stays, and the keys stay with it." [I] A locked owner blocks all of these.
- **Old versions stay callable by anyone. This is a new D5 precondition.** [V] https://app.outlayer.ai/docs/projects: "you can execute any specific version using the version_key parameter … To stop an old version from being called on an ordinary project, remove it (remove_version)". All versions share storage: "All versions of a project share the same storage namespace." [I] Before locking, remove every version except the audited one. Otherwise any caller can run an older build against the same storage and author secrets. A One-build-only rule on the secret would block the secret but **not** storage access. Add this check to the testnet reproduction.
- **Pin the survey key as an author secret.** [V] https://app.outlayer.ai/docs/secrets: "A project can also carry one of its author's own … decrypted into every run without the caller naming anything", "Its access condition is who may run the project at all", "a name defined on both sides refuses that call". The manifest can name the owner: [V] `worker/src/connector_manifest.rs:211-214` uses `author_secrets.owner`, falling back to the project owner. [I] The (owner, profile) is then inside the wasm hash, so a client verifying GetPublicKey's attestation knows which secret row produced the key. Without it, a stranger stores their own `PROTECTED_*` under our project (allowed, per `secrets.rs:2004-2011`) and calls GetPublicKey with `secrets_ref` naming it. That yields an attested public key from our wasm hash that is not ours. Also have the module read **only** author-secret names. A caller's extra names are "added as the caller's" [V secrets page].
- **Signatures needed during normal operation:**
  - **Payment-key top-ups** are by the key's owner (the service account, not the project or secret owner). [V] https://app.outlayer.ai/docs/payment-keys `ft_transfer_call … --accountId alice.near`. [I] Compatible with D5. The service account stays live, and it also owns the storage cell (Q2).
  - **Project storage deposit** is paid at creation, "refunded when deleted" [V projects]. No recurring charge was found.
  - **Secrets:** no renewal unless a `ValidUntil` is set [V secrets page, access conditions]. Do not set one.
  - **Keystore rotation:** CKD is deterministic. [V] https://app.outlayer.ai/docs/trust-verification: "when the keystore restarts or upgrades, it recovers the same master secret — all previously encrypted secrets remain accessible."
  - **Version expiry:** none found in `contract/src/projects.rs`/`types.rs` [I from grep].
- **MPC Vaults make the trust picture worse; avoid them.**
  - [V] https://app.outlayer.ai/docs/vaults: "The parent account can exit at any time without DAO involvement" and "anyone holding a FullAccess key on the unlocked vault can submit a fresh request_app_private_key … and arrive at the same 32-byte master". That master decrypts the vault's secrets offline ("decrypt-secret --master …").
  - [I] A survey key under a vault is readable by the vault parent after the exit window, which violates D1.
  - With a locked parent, the vault degrades. [V] Same page: "the vault contract caps registered TEE keys at 32 … Parents can retire stale keys with clear_unused_tee_keys" and "If the vault runs out of NEAR … the vault stops working". [I] A locked parent can't retire keys, so the vault eventually stops after repeated keystore upgrades.
- **The DAO-governed default keystore is a trust root wider than D5 states.**
  - [V] `keystore-dao-contract/src/lib.rs:653` `add_approved_measurements` → `self.assert_owner()`; `:850` `add_dao_member` → `self.assert_owner()`.
  - `:987-991` `request_key` requires only `predecessor == current_account`: "must be called via an approved-keystore access key on this contract".
  - [I] The keystore-DAO **owner** alone sets measurements and membership. Any access key on `dao.outlayer.near`, including a full-access key if one exists, can request the default master from MPC. D5's disclosed trust root should name the keystore-DAO owner and the DAO account's access keys explicitly, not only "dao.outlayer.near approves builds".

---

## Q6. Other contradictions and simplifications

- **D2 ("participation hidden"):** partially undermined by the public job log (Q3). Disclose that the count and timing of submissions are public, or add dummies and delays. [I]
- **D1 ("every link that touches plaintext must run in attested code"):** holds, but add the requirement that respondent IPs and timing go only to OutLayer, never through a relay we run. [I] A relay we operate would see IPs, which breaks "neither … we … may see who participated".
- **Shared payment key abuse [I]:** a key handed to every respondent agent is effectively public. Anyone can burn its balance (1000 req/min per key, 10 concurrent) [V payment-keys]. Set `project_ids` to the survey project and `max_per_call` [V payment-keys "Key Restrictions"], and monitor `/public/payment-keys/.../balance`.
- **Time:** HTTPS runs have no block time. [V] https-api: "`NEAR_BLOCK_TIMESTAMP` | Block timestamp | """. This confirms the prior research. Deadline enforcement still rests on one-shot plus the host clock.
- **The survey key must still be `PROTECTED_*` plus our own ECIES** (Q2). The only change is storing it as an **author secret**.
- **`GET /calls/{call_id}`:** API.md lists auth "—", but the live OpenAPI requires a payment key or Bearer token [V `https://api.outlayer.ai/openapi.json`: `security: PaymentKeyAuth | BearerAuth`]. [I] Either way, keep outputs non-identifying.

---

## Corrections to prior research (`near-confidential-stack.md`)

1. **§2.4 / §3 link 3 / §4(g):** "Atomic ops include `set-if-absent` and CAS" was read as applying to `@worker`. Wrong. `@worker` has only `set-worker`/`get-worker` [V storage.wit:103-120]. "keyed by a hash of the respondent's identity for dedup" in `@worker` cannot be atomic or enumerable. Use the shared caller cell (Q2).
2. **§2.4:** "Raw records are readable by the operator" was presented as a general storage property. It applies to `-raw` records. Encrypted records hide the name but store an unsalted SHA-256 of it [V api_support.rs:1617-1618]. "Operator" means OutLayer's coordinator DB, not the project owner [V mod.rs:1-23].
3. **§2.6:** the claim that attestation "commits to input, output, wasm hash, caller, payment, and project" is correct per source (`tdx_attestation.rs:150-152`). The docs page tee-attestation shows a stale formula without them. More importantly, these records are **public by id** and the job list is **public and enumerable** [V-live]. The prior research's "only hashes of HTTPS payloads are stored" is true but missed that caller and timing are published.
4. **§4(c):** "a trial `Authorization: Bearer wk_...` from `POST /register`" as a non-transaction mode for agents is wrong for our project. Trial calls are connector-only [V payment-keys, connectors]. Also, `NEAR_SENDER_ID` may be the *bound* account under `use_bound_identity`; `NEAR_USER_ACCOUNT_ID` stays the payer [V main.rs:1455-1464].
5. **§4(f), §5.4:** "HTTPS mode keeps participation off-chain, but the closed-source coordinator still sees the payment-key owner and timing" understates it. The coordinator **publishes** owner, project and timing at `/public/jobs` and `/attestations/{id}` [V-live].
6. **§2.8:** "The TEE generates a master secret" in private-dao-example. The architecture text says so, but the deploy steps generate it locally with `openssl` and store it as a manual secret [V examples page]. It is WASI P1, and its dummy votes hide whether a member voted, not who the members are.
7. **§2.3 / §4(a):** missing a vector. Any account can store a project-bound secret for any existing project [V secrets.rs:2004-2011], and callers choose `secrets_ref`. A survey key read from a caller-named profile can be substituted. Use `author_secrets` with an explicit owner.
8. **§4(b) hardening option 1:** a locked owner is necessary but not sufficient. Old versions remain callable via `version_key` with shared storage [V projects page]. Remove them before locking.
9. **§4(b) option 2 / MPC vaults (implicit in "own it from a multisig"):** the vaults page shows a vault parent can recover the master and decrypt secrets offline [V vaults]. Never put the survey key under a vault.
10. **§6 architecture, "GITHUB_TOKEN (One-build-only)":** under a locked secret owner (D5) the token can't be renewed or re-pointed. Prefer public-storage report plus maintainer-side posting (Q4).
11. **§2.7:** the docs do document NEAR RPC host functions; the prior research said "no first-party NEAR AI integration", which is still correct. The access-key lookups (`view-access-key[-list]`) exist only in source (`world.wit:20,24`), not in the docs page.
12. **Unknowns list, "`@worker` size limits":** still undocumented, but practical ceilings exist: the 25 s `list-keys` keystore budget [V client.rs:437] and 100 RPC calls per run [V wasi_p2.rs:446].

---

## Unknowns

- Whether the coordinator retains HTTPS request bodies, IPs or payment-key usage logs, and for how long. It is closed source; the docs mention only "Usage history (audit log)".
- Whether `/public/jobs` supports filtering by project. In a quick probe a `project_id` param didn't apply, but pagination with `offset` does. Its retention is also unknown. Whether job ids or call ids can be hidden per project is undocumented. It appears not.
- Whether worker or keystore debug logs (which print storage key names) leave the CVM, and to whom.
- Storage value-size limits and storage cost on the coordinator side.
- Whether `dao.outlayer.near` holds any full-access key, and who owns the keystore-DAO contract. Both determine who can request the default master via `request_key`.
- The production value of the RPC proxy URL (FastNEAR per docs), and whether the 100-call cap is ever raised.
- Whether a module can make an authenticated nested `/call` to the GitHub connector from inside a run, and whether that is supported.
- Whether the WASI clock inside workers can be trusted for deadlines (carried over).
- Whether NEAR offers a verifiable state proof for access keys that would let the TEE check `account → key` against a block header from several RPCs, like the Ethereum examples' RPC quorum ([V] examples page: "Every read requires an N-of-M RPC quorum (min_agree)"). [I] A quorum of several NEAR RPCs would reduce trust in any single provider's *answer* but would *increase* the number of parties who see the account.
