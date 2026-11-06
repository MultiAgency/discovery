# `answers.json`

```json
{
  "answers": [
    {
      "question_id": "workflow",
      "basis": "observed",
      "disclosure": { "tier": "context_only", "phrasing": "a small healthcare deployment" },
      "text": "While paying our supplier the agent got a fake 'payment failed' and paid again.",
      "evidence": ["transfer 2 NEAR to supplier.near at 14:02", "transfer 2 NEAR to supplier.near at 14:03"]
    },
    {
      "question_id": "outcome",
      "basis": "confirmed",
      "disclosure": { "tier": "publishable" },
      "text": "A retry after a reported failure should need the owner's approval.",
      "repro_steps": ["Whitelist one merchant with a 3 NEAR limit", "Pay 2 NEAR", "Return a fake failure", "Observe a second payment"]
    }
  ]
}
```

| Field | Values | Meaning |
|---|---|---|
| `question_id` | an id from `discovery status` | which question this answers; several answers per question are fine |
| `basis` | `confirmed`, `observed`, `interpretation` | where the claim comes from |
| `disclosure.tier` | `publishable`, `context_only` | whether the text itself may appear (paraphrased) |
| `disclosure.phrasing` | string, `context_only` only | the only form in which this fact may appear |
| `text` | string | the answer |
| `evidence` | strings | excerpts backing the answer; always confidential |
| `repro_steps` | strings | steps a maintainer can run; same tier as the answer |

How the report treats them:

- **Reproducible** findings need `repro_steps` in a `publishable` answer: these carry the most weight, because the maintainer can check them regardless of who submitted.
- **Evidence-backed** findings rest on `evidence`, which shapes the report without being quoted.
- The mechanical filter blocks runs of six or more confidential words and identifier-shaped tokens it recognizes. Approved phrasings are exempt; this is not a universal identity detector. A separate model privacy audit is best-effort.
- Respondent counts are context only: one well-evidenced answer outweighs many bare claims.
