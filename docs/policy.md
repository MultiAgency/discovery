# Service policy

What the service promises and what it relies on. The skills and the invitation
link here rather than restating it.

## Authorization

The maintainer approves the issue, questions, deadline and restrictions. A
personal agent gets its human's approval of the final answers, their basis and
their disclosure tiers before submitting. Skills guide that approval; the service
cannot prove a human gave it. The full survey definition is public, including
withheld terms, even if the issue is private.

## Disclosure

Only the report is released; no operation returns individual responses.
Responses are encrypted to a key that only the locked, reviewed release can use.
Respondents verify that key on-chain before encrypting to it.

- Each answer is `publishable` or `context_only`. A context-only fact can
  surface only through the respondent's approved phrasing; without one it may
  shape the synthesis but must not be stated.
- Organizer restrictions (`no_direct_quotes`, `withheld_terms`) can only tighten
  publication.
- Each finding must cite authorized answers. Its basis (confirmed by a human,
  observed by an agent, or interpretation) comes from those answers, not from
  the model. Citations stay private. A citation proves permission to draw on an
  answer, not that the answer supports the statement; the summary, disagreements,
  open questions and acceptance criteria cite nothing. Citations the model puts
  on proposed acceptance-criteria changes are accepted and discarded; only
  findings carry a basis.
- Mechanical checks block runs of six or more confidential words, recognized
  identifier-shaped tokens, withheld terms and stated respondent counts (a
  quantity word shortly before a word such as respondents or responses). Survey wording and approved
  material are exempt. The checks cannot recognize every personal detail or
  semantic disclosure. A separate model audit is best-effort.
- The count check is a backstop for the model's instruction never to state
  counts: it misses semantic forms ("two humans", "most were synthetic"), and
  it can reject harmless technical phrases ("two response formats"), which
  costs a retry.
- A rejected draft is regenerated, up to three attempts in all. If every attempt
  fails, only a fixed withholding notice is released. Rejection reasons
  (including format errors, which go back to the model inside the enclave) and
  provider errors are never published.

There is no anonymity promise. Distinctive approved details can identify someone.
OutLayer's public job log shows submission counts and times under the shared
service account. Anyone with the payment key can contribute, so counts are
context, not proof of usage or a vote. Respondents must not announce their
participation.

## Collection and retention

The deadline is the earliest finalization time. Collection stays open until the
organizer finalizes; finalization closes admission and freezes the collection,
and an in-flight submission is included only if its write wins. Nothing is
scheduled.

Responses are immutable, with no editing or withdrawal. Identical answer
batches contribute once, including re-encrypted retries and identical batches
from different agents.

Each response is at most 4,092 bytes of serialized JSON, padded to one 4 KiB
size (including its length prefix). A survey admits 18 registrations, so at
most about 74 KB of responses; beyond that, submissions fail with "survey is
full". There is no sampling. Retries of admitted responses still succeed.
Identical submissions that race, or a retry after an interrupted submission,
may use another place each time; they still contribute once.

The limit comes from storage, not the model. On testnet, OutLayer failed
finalization runs on their 126th storage call, which hung until the 30-second
client timeout; the cause is unresolved. Finalization costs 8 storage calls
plus 3 per registration, or 5 for a slot whose delayed writer inserts while it
is being frozen. At 18 registrations it needs 62 calls, and 98 if every slot
races, a quarter below that point. Finalization makes up to six model requests
(three drafts, three audits) in one 180-second job.

Flooding with the public payment key can fill the survey early instead of
admitting an unbounded batch that blocks finalization. Recover by creating a new
survey; abuse handling remains open and there is no Sybil resistance.

Encrypted responses are kept until a finalization succeeds; they have no
independent expiry. Once the report is stored, response contents and the survey
key are replaced with sealed empty markers. Retries resume this cleanup without
producing a second report. Registration metadata and markers remain to fence
delayed writes. Storage backups and physical erasure are not verified.

One report is stored per survey. Operator retries and competing finalizers may
run inference more than once. Model attestation evidence is not kept.

## Trust

The service relies on OutLayer's worker and keystore governance, NEAR AI's
attested model, and the NEAR RPC used for verification. The CLI checks that the
creation and finalization transactions came from the organizer, ran the
registered project and build, and that at that block the build was the project's
only version and its owner was an initialized account with no access keys or
contract code. Before sending any response content, the module checks the
gateway's and model's Intel TDX quotes, NVIDIA's GPU verdict and the model's
attested signing key, all bound to a fresh nonce. It encrypts each request to
that key and uses a reply only after the model's signature over the exact
request and reply checks out.

The MVP trusts NEAR AI to manage which software runs on that attested hardware.
NEAR AI deploys model containers frequently, and applications choose their own
[verification policy](https://docs.near.ai/cloud/verification/reference/verification-policy).
Hardware attestation does not establish that independently reviewed model code
processed a response; this service does not check that. Our own code is
independently pinned: the locked OutLayer release enforces contributor
permissions, publication checks and recoverable cleanup.

## MVP versus concept

- The organizer finalizes after the deadline; nothing is scheduled.
- The organizer posts the verified report with `gh`; the service and CLI make
  no GitHub calls, though every survey targets a GitHub issue.
- Responses stay encrypted until finalization succeeds, with no independent expiry.
- Confidentiality does not guarantee anonymity; approved details may identify someone.
