# Confidential surveys for agents — hackathon MVP

**A confidential survey service for agents. Organizations’ agents request context; users’ personal agents respond on their behalf; the service privately analyzes responses and returns findings.**

Working concept and hackathon MVP · November 4, 2026.

The [main demo](demo.md) lets anyone follow a shared invitation link to try
Discovery with their agent, submit approved feedback privately, and see
permitted findings become actionable GitHub issues. The invitation can live
anywhere; each survey currently uses a GitHub issue as its report destination.

## 1. Problem

Organizations need useful context from people without requiring them to expose their individual answers or underlying personal data to the survey organizer.

Our first use case is GitHub issue enrichment for FOSS maintainers. Personal agents make it easier for people to report problems and request features in free and open-source software. Maintainers receive more useful signals, but also more noise. Issues often lack the context needed to assess their importance or understand the requirements and acceptance criteria. Software factories need evidence about which problems matter, whose workflows are affected and what a useful solution would require.

## 2. Solution

An organizing agent creates a survey, personal agents contribute authorized responses, and the organizer finalizes the survey to close collection and start confidential analysis. The deadline is the earliest finalization time; collection stays open until finalization. The organizer receives a report under agreed disclosure rules.

For GitHub issue enrichment, responses explain workflows, use cases, obstacles, workarounds, relevant personal circumstances and desired outcomes. The report is posted to the issue, where the maintainer’s existing agent uses it to refine requirements and prioritize work.

## 3. Target users and assumptions

The initial market is local and self-hosted FOSS, particularly personal-agent products. The same core service could later support marketing research, community management and internal team discovery.

- **Existing agents on both sides:** maintainers already operate agents or software factories; users have personal agents with relevant context. They need tools and skills that fit those setups.
- **Participation is motivated by unmet needs:** users and their agents will find relevant issues and respond because better evidence may increase the likelihood that their needs are addressed.
- **Agents provide the interface:** personal agents gather authorized context and interview their users when needed, using their existing text or voice tools to prepare survey responses. This removes the need for us to build an end-user frontend or interview experience.
- **Research methods remain flexible:** organizing agents craft surveys according to their priorities, guided by our skill. The GitHub demonstration focuses on maintainer research.

These are working assumptions to test through the prototype and subsequent use.

## 4. User journey

The first application demonstrates the shared survey workflow through GitHub:

1. **Create a request.** The maintainer agent uses the skill and CLI to prepare a machine-readable survey with an associated issue, deadline and disclosure terms. The CLI prints unsigned creation arguments for the organizer to sign with `near`. A link is posted on GitHub.
2. **Prepare answers.** The personal agent follows the respondent skill, draws on authorized context and interviews its human when useful. It distinguishes confirmed answers from observations or interpretations.
3. **Submit privately.** The CLI sends the authorized response through the confidential survey service. Individual responses and participation are not posted on GitHub.
4. **Process the batch.** After the deadline, the organizer finalizes the survey, closing collection and triggering confidential processing, which synthesizes the responses and applies publication restrictions.
5. **Publish and act.** The organizer posts the verified report, or a withholding notice, on the issue. The maintainer agent considers it alongside other evidence when refining or prioritizing the work.

## 5. MVP scope

| Component | Responsibility |
| --- | --- |
| **Survey-organizer skill** | Guide survey design and interpretation, with maintainer research as the first application |
| **Respondent skill** | Guide authorized disclosure, human clarification and response preparation |
| **CLI** | Print unsigned creation and finalization arguments, inspect surveys, submit responses, check status and retrieve reports |
| **Confidential processing service** | Protect transfer and storage, manage collection and finalization, analyze responses and enforce publication rules |
| **GitHub integration** | The organizer posts survey links and verified reports on existing issues with `gh`; the service makes no GitHub calls |

Survey creation, response submission and report retrieval are independent of GitHub. GitHub is the only integration demonstrated in the MVP.

The MVP integrates with existing agents. A dedicated end-user frontend, new agent harness, built-in interviewer and prioritization engine are outside scope.

Telemetry collection and usage verification are outside scope. Future adapters could join product-specific context to answers inside confidential processing. Usage data would inform interpretation rather than automatically determine priority; occasional users and people who stopped using a product may reveal important unmet needs.

## 6. Privacy and NEAR architecture

OutLayer runs the service and its encrypted storage in TEE-backed workers. NEAR AI provides confidential model inference. The [service policy](policy.md) states exactly what each is trusted for.

The intended boundary is that survey organizers receive published findings without access to individual answers or underlying personal context. Transfer, storage, model calls and application logging must preserve that boundary. Intermediate data should be discarded under the agreed retention policy.

Reports should preserve useful themes, disagreement, uncertainty and implications for the issue while withholding identifying details. Aggregation alone does not guarantee anonymity. Survey instructions cannot override contributor permissions or publication restrictions.

**Primary hackathon fit: Privacy & Private AI.**

## 7. Success criteria and open questions

The demo should complete the full request-to-report cycle and show that the report improves understanding of an existing issue—for example, by changing an acceptance criterion or exposing an incorrect assumption. It should also demonstrate that identifying material is withheld under the publication rules.

The [service policy](policy.md) sets the MVP's authorization, privacy promise, retention, abuse handling, synthesis checks and withholding outcome.
