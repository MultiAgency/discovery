> Historical material. The current scope is [the November 4 concept](../concept.md); these documents do not define current features or operating instructions.

# Confidential surveys for agents — hackathon MVP

**A confidential survey and product-review service for agents. Maintainers' agents request evidence. Respondents' agents answer from real use, on their own behalf or their owners'. The service analyzes the responses inside attested confidential compute and publishes only findings that pass the agreed disclosure rules.**

Working concept and hackathon MVP · November 5, 2026 (revised after the requirements pass). Product name to be decided.

## 1. Problem

Organizations need useful context from people and their agents, without requiring them to expose individual answers, underlying personal data or the fact that they took part.

Our first use case is GitHub issue enrichment and product review for maintainers of agent-facing FOSS tools. Agents are increasingly the actual users of CLIs, libraries, connectors and APIs, and they are also how people report problems. Maintainers receive more signals, but also more noise. Issues often lack the context needed to assess importance or to write requirements and acceptance criteria, and nobody checks whether a fix works for the people who had the problem before it's merged.

The most valuable evidence is the hardest to share publicly:
- what happened in real deployments, with real data, counterparties and configurations;
- organizations' usage and agent stacks;
- security-sensitive failures, such as an agent tricked into paying twice.

## 2. Solution

An organizing agent creates a survey with fixed questions, deadline and disclosure rules. Respondents' agents contribute authorized responses. After the timed collection period, the service analyzes the responses confidentially. The organizer receives a report under the agreed disclosure rules.

The same mechanism covers the whole change lifecycle:

1. **Issue survey:** evidence about workflows, use cases, obstacles, workarounds, relevant circumstances and desired outcomes.
2. **Report:** themes, disagreement, uncertainty, proposed acceptance criteria and suggested tasks, posted to the issue. The maintainer's agent turns it into issues, tasks and project boards.
3. **Product review:** when a PR implements the change, respondents' agents test the build against their real setups. They report findings in code-review style: severity, the acceptance criterion concerned, and a failure scenario.
4. **Review report:** a verdict per acceptance criterion, posted to the PR, so the maintainer merges with evidence.

## 3. Target users and assumptions

The initial market is agent-facing FOSS tools and their users' agents, starting with the NEAR and OutLayer ecosystem. The same core service could later support marketing research, community management and internal team discovery.

- **Existing agents on both sides:** maintainers already operate agents or software factories. Respondents have agents with relevant context and working history. They need tools and skills that fit those setups.
- **Agents are respondents, not only interviewers:** an agent can answer from its own working history, with its owner authorizing what is shared. Personal agents still interview their humans when useful.
- **Real-use evidence is the confidential core:** guided walkthroughs, run in a sandbox, give comparable answers to test a hypothesis. Privacy matters most for evidence from real deployments, organizations and security-sensitive failures.
- **Participation is motivated by unmet needs:** respondents take part because better evidence makes it more likely their problem gets fixed, and that the fix actually works for them.
- **Agents provide the interface:** no end-user frontend and no hosted interviewer.

These are working assumptions to test through the prototype and subsequent use.

## 4. User journey

1. **Create a request.** The maintainer's agent uses the organizer skill and CLI to create a survey: questions or walkthrough, target issue or PR, deadline and disclosure rules. The enclave generates the survey key and fixes the definition. Both are covered by one attestation. A link is posted on GitHub.
2. **Prepare answers.** The respondent's agent follows the respondent skill. It verifies the attestation and the survey definition, and treats the organizer's script as untrusted. It draws on authorized context or real-use history, running walkthroughs or PR builds only in a sandbox or on a copy of the setup. It marks each item:
   - **confirmed**, **observed by an agent** or **interpretation**;
   - **`publishable`**, or **`context-only`** with a respondent-approved phrasing.
3. **Submit privately.** The CLI signs the response with the respondent's local key, optionally tied to an OutLayer custody wallet, then encrypts it to the survey key and submits it through a shared service payment key. Individual responses and participation never appear on GitHub. The respondent should not announce participation.
4. **Process the batch.** After the deadline, which is enforced on-chain, the enclave runs once:
   1. decrypts the responses and merges near-identical ones;
   2. synthesizes with a NEAR AI Confidential TEE model;
   3. applies the publication checks;
   4. outputs the report as attested output;
   5. discards the responses.
5. **Publish and act.** The maintainer's agent posts the report on the issue or PR, using its own token or OutLayer's GitHub connector with human confirmation. Anyone can check the posted report against the attestation. The maintainer's agent weighs the report with other evidence when refining requirements, creating tasks and boards, or deciding to merge.

## 5. MVP scope

| Component | Responsibility |
| --- | --- |
| **Survey-organizer skill** | Design surveys, walkthroughs and product reviews. Interpret reports. Turn reports into issues, tasks and boards with human approval |
| **Respondent skill** | Authorized disclosure, disclosure tiers, evidence handling, sandboxed walkthroughs and PR builds, treating organizer scripts as untrusted |
| **CLI** | Create, inspect and finalize surveys. Verify attestations and definitions. Encrypt, sign and submit. Check status. Retrieve and verify reports |
| **Confidential processing (OutLayer module)** | `InitSurvey` (survey keypair, fixed definition), `Submit` (verify, deduplicate, store), `Finalize` (deadline, synthesis, publication checks, discard) |
| **GitHub integration** | Post survey links and reports on issues and PRs, through the maintainer's agent |

Survey creation, response submission and report retrieval are independent of GitHub. GitHub is the only integration demonstrated in the MVP.

**Outside scope:**
- a dedicated end-user frontend, new agent harness, hosted interviewer and prioritization engine;
- telemetry, session replay and usage verification;
- flood-resistance gates for respondents.

Future work: survey-set gates on anchored identities (deposit, account age, named account), enclave-verified respondent attributes (e.g. "has used this feature"), and a minimum-respondent threshold. Usage data would inform interpretation rather than determine priority.

## 6. Privacy and NEAR architecture

**The promise:** neither the organizer nor we, the operator, can see individual responses or learn who participated. The published report passes the agreed disclosure rules. This is private redaction, not anonymity in a crowd: a report may draw on a single response.

- **Confidential processing:** an OutLayer WASI module running in Intel TDX. Each release is its own project, owned by a subaccount whose keys are deleted after deployment, so nobody, including us, can change the code. The survey's private key is sealed with an encryption key bound to the project and our service account, and no other project can derive it. We can block finalization (availability), never read responses (confidentiality).
- **Agreed rules:** the survey definition is fixed at creation and covered by the attestation. Each response is bound to it. The organizer can't change questions, deadline or rules afterwards. Survey instructions cannot override contributor permissions or publication restrictions.
- **Timed collection:** finalization goes through a NEAR transaction, so the deadline is checked against a trusted block timestamp.
- **Model calls:** NEAR AI Confidential TEE models only, using the canonical model ID, with end-to-end encryption where feasible. The module can reach only NEAR AI. No GitHub credential enters the enclave.
- **Storage and transfer:** responses are encrypted on the client. They're stored in OutLayer's encrypted storage under unguessable names, so OutLayer's front end and storage see only ciphertext.
- **Publication checks:**
  - a mechanical check, which is the guarantee: no verbatim overlap with `context-only` text or with other respondents' text, and no identifiers;
  - a model audit, best-effort;
  - on failure: regenerate once, then publish only "could not be released," and discard the responses.
- **Reliability (D11):** findings are weighted by evidence (reproducible, evidence-backed, reported), not by headcount. Reports state response and merge counts and their own limits.
- **Disclosed limits:**
  - OutLayer's public job log shows submission counts and timing;
  - combinations of approved phrasings can identify someone;
  - floods are weakly resisted;
  - the trust root is OutLayer's keystore and administrators, plus NEAR AI's TEE. Changes to it are visible on-chain.

**Primary hackathon fit: Privacy & Private AI.**

## 7. Success criteria and open questions

**The demo:** the full issue → report → PR → product review cycle on `MultiAgency/discovery`'s own issues and PRs, with the agents building this project as respondents. A report should change an acceptance criterion or expose a wrong assumption, a product review should give a verdict on a PR against those criteria, and the demo should show that identifying material (local paths, credentials, agent setups) is withheld. Limit: organizer and respondents are largely the same people, so this exercises the loop but doesn't demonstrate the privacy guarantee to an outside party.

**Settled by the requirements pass:** authorization, the precise privacy promise, retention, response-abuse handling, synthesis checks, the outcome when a report can't safely be released, timed collection and agreed rules.

**Must be verified on testnet before building on it:**
- **Locked releases:** a project with a keyless owner still runs; `add_version` fails; the key and storage are the same over HTTPS and on-chain for our service account; `set-if-absent` protects the keypair.
- **Signing:** ed25519, HKDF and NEP-413 serialization fit in the module, and custody-wallet signatures verify there.
- **Public log:** custody signing doesn't appear in OutLayer's public job log.
- **Deadline:** `NEAR_BLOCK_TIMESTAMP` is available in on-chain runs, and OutLayer issues #32 and #33 don't block on-chain or HTTPS calls.

Supporting material: `research/near-confidential-stack.md`, `research/outlayer-docs-review.md`, `docs/decisions.md`.
