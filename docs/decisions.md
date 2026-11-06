# Current service policy

The [November 4 concept](concept.md) defines the MVP. These decisions resolve its
open questions for the current implementation.

## Authorization and scope

The maintainer approves the survey questions, issue, deadline and restrictions.
The personal agent gets authorization for the final answers, basis, disclosure
tiers, approved phrasings and evidence before submitting. Existing explicit
authorization applies. Skills guide approval; the service cannot prove that a
human actually approved a response or that a respondent used the product.

Create issue surveys only. Personal agents gather context or interview humans
with their existing tools. Human-confirmed answers, agent observations and
interpretations remain distinct in the report. The output is a shared research
report with findings, disagreements, open questions and proposed acceptance
criteria. Respondents use local signing keys derived separately for each survey.
Usage verification and prioritization are outside scope.

## Confidentiality and disclosure

Only the report is released; no operation returns individual responses. The
chosen implementation also protects responses from the service operator by
pinning reviewed code in a locked OutLayer project. Respondents verify the
survey definition and key against that release before encrypting.

Each answer is publishable or context-only. Context-only facts can surface only
through respondent-approved phrasings; without one, they may inform synthesis
but must not be stated. Evidence is always confidential. Organizer restrictions
can tighten publication rules, not override respondent permissions.

Mechanical checks block runs of six or more confidential words, recognized
identifier-shaped tokens and organizer-withheld terms. Survey wording and
approved public material are exempt. These checks do not recognize all personal
information or semantic disclosures. A separate model audit is best-effort.
Rejected drafts are regenerated once; a second failure releases only a fixed
withholding notice. Private rejection reasons are not published. Development
builds with diagnostics are for synthetic data only.

Do not promise anonymity. Distinctive approved information can identify someone.
OutLayer observes network metadata and publishes submission activity/counts and
timing under the shared service account. Participants must not announce their
participation on GitHub. Identities are free to create; counts are context, not
proof of usage or a vote on importance.

## Collection and retention

The deadline is the earliest permitted on-chain finalization time. HTTPS
submissions may still arrive after it until finalization starts. Finalization
closes admission and freezes a stable collection; in-flight submissions may be
included or rejected according to which write wins. Edits use the same local
identity and replace older signed submissions while collection is open.

Encrypted response contents are retained until successful finalization releases
or withholds a report. There is no independent expiry if finalization never
completes. Operators must retry failed finalizations. After report persistence,
cleanup replaces response contents and the survey secret with sealed empty
markers; retries resume cleanup without generating another released report.
Opaque registration metadata and tombstones remain to fence delayed writes.
Underlying storage backups and physical erasure are not independently verified.

The report and content-free attestation receipts are retained. One report is
published per survey; transport retries or competing finalizers can execute
inference more than once. No exactly-once inference claim is made.

## Trust and release policy

The service relies on OutLayer's worker/keystore governance, NEAR AI's approved
confidential workload, and the configured NEAR RPC and contract for verification.
The CLI checks on-chain report provenance, not an independent inference receipt.
Model hardware evidence, GPU verdicts and signed turns are checked in the module
before results are used. Approved model configurations are baked into the build;
[workload approval](workload-approval.md) must precede a new release.

Existing projects are immutable. New code and formats ship in a newly reviewed,
locked project. Registry entries retain their historical project and build hash.
