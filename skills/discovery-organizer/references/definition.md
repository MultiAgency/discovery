# `definition.json`

```json
{
  "version": 1,
  "kind": "issue_survey",
  "title": "Per-destination payment allowance: what should it guarantee?",
  "target": { "repo": "owner/name", "number": 42 },
  "deadline_ns": 1794000000000000000,
  "questions": [
    { "id": "workflow", "prompt": "What were you or your agent doing when this came up?" },
    { "id": "obstacle", "prompt": "What went wrong, and what did you try?" },
    { "id": "outcome", "prompt": "What would a fix need to guarantee for you?" }
  ],
  "rules": { "no_direct_quotes": false, "withheld_terms": [] }
}
```

| Field | Notes |
|---|---|
| `version` | always `1` |
| `kind` | `issue_survey` |
| `target` | the existing issue the report is about |
| `deadline_ns` | NEAR block time in nanoseconds; five days from now: `echo $(( ($(date +%s) + 5*86400) * 1000000000 ))`. Days for real surveys |
| `questions` | 2–5, each with a unique `id`. Ask about experience and needs, not identity |
| `rules.no_direct_quotes` | `true` forbids quoting even publishable answers |
| `rules.withheld_terms` | names nobody marked confidential that must never appear (a customer, a codename). Matched as substrings, so keep them specific. Respondents' own identifiers are already blocked; leave this empty when nothing applies |

Good questions invite evidence a maintainer can act on: workflows, obstacles, workarounds, what a fix must guarantee, what would break. Ask what the person was trying to achieve, what happened, what they tried and what a useful outcome would look like.

The definition is fixed at creation and hashed into the survey id; respondents see all of it, title included. The `survey` Action validates it, and checks that `target` is the existing issue it was posted on, before anything goes on-chain.

The deadline is the earliest finalization time. HTTPS collection stays open until
finalization begins; disclose that behavior. Responses are retained encrypted
until finalization can release or withhold a report, then their contents and the
survey key are replaced with empty sealed markers. The full terms are in
[the service policy](../../../docs/decisions.md).
