---
name: discovery-organizer
description: Request confidential context about an existing GitHub issue and interpret the resulting report. Use when a maintainer needs evidence about affected workflows, obstacles, circumstances or desired outcomes.
---

# Organizing a confidential survey

Help a maintainer understand an issue through context that users would hesitate
to publish. Existing personal agents prepare authorized responses; the service
returns findings, without giving the organizer individual answers.

## Prepare the request

Read the issue and identify the decision the maintainer cannot yet make. Ask
about actual experiences, obstacles, workarounds and desired outcomes. Keep
questions neutral; distinguish the user's problem from a proposed solution.
Use [the definition reference](references/definition.md) to write the fixed
questions, target issue, deadline and disclosure restrictions.

Show the owner the definition and the disclosure/retention terms in
[the service policy](../../docs/decisions.md). Posting the request spends the
repository's survey budget. Obtain approval for the definition and posting;
existing explicit authorization applies. Do not create a PR review or a new
agent workflow as part of the survey.

## Publish the request

With write access to the repository, post one JSON definition block:

````markdown
/survey

```json
{ ...the approved definition... }
```
````

```bash
gh issue comment <number> --repo <owner/name> --body-file survey-comment.md
```

The repository's `survey` Action creates the survey and posts the announcement,
or reports an error. Its `finalize` Action posts the report after the deadline.
Repository operators configure these Actions; maintainers need no wallet or
model credentials. For repositories without the Actions, operators can use the
CLI following [operations](../../docs/operations.md).

## Interpret the report

Check its provenance with `discovery report <survey> --finalize-tx <transaction>`
using the pinned release settings. A withheld report contains no findings and
no private rejection reasons. Do not infer that no one had a problem.

Help the maintainer understand affected workflows, consequences, disagreement,
uncertainty and desired outcomes. Distinguish human-confirmed evidence, agent
observations and interpretations. Counts do not establish importance or verified
usage. Proposed criteria and tasks are suggestions for the maintainer's existing
agent; they are not automatically applied or prioritized by this service.

Identify whether the report changes an acceptance criterion or corrects an
assumption. Obtain the owner's authorization before editing the issue or taking
other GitHub actions. Never publish private respondent material or participation.
