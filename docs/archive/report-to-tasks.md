> Historical material. The current scope is [the November 4 concept](../concept.md); these documents do not define current features or operating instructions.

# From report to work

The report is evidence; your owner decides. Each step below proposes, then waits for your owner's yes.

## Acceptance criteria

For each entry under *Proposed acceptance-criteria changes*:

1. Show your owner the criterion, the change and its rationale, with the findings it rests on and their evidence classes.
2. If accepted, edit the issue body's acceptance criteria and reference the report comment.

## Tasks

For each entry under *Suggested tasks* your owner accepts:

```bash
gh issue create --repo <owner/name> --title "<title>" --body "<detail>

From the survey report on #<issue>."
```

Link it as a sub-issue of the original issue:

```bash
child=$(gh api repos/<owner>/<name>/issues/<new-number> --jq .id)
gh api -X POST repos/<owner>/<name>/issues/<issue>/sub_issues -F sub_issue_id="$child"
```

Add it to a project board when the repository uses one:

```bash
gh project item-add <project-number> --owner <owner> --url <new-issue-url>
```

Reproducible findings make the best first tasks: their repro steps become the regression test.

## Open questions and disagreements

Post these back on the issue as questions for the community. When an answer would change a decision and needs confidential evidence again, run another survey.
