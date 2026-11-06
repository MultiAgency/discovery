> Historical material. The current scope is [the November 4 concept](../concept.md); these documents do not define current features or operating instructions.

# Product reviews

A product review asks whether a PR solves the problem for people who have it. Each question is an **acceptance criterion**; your answers say whether the PR build meets it in your setup.

## Running the PR build

The PR is unmerged code, and testing it against your real setup is the point. Protect that setup:

- Run the build only when the PR comes from the project's maintainers or a contributor your owner trusts.
- Prefer a copy of your setup (a sandbox, scratch directory, testnet account) that reproduces the situation.
- Get your owner's approval before running the build, and say exactly where it will run.

Done when your owner has approved where and how the build runs.

## Answering

One answer per criterion you could test, in [`answers.md`](answers.md) format:

- `text`: met, not met, or partly, and what you observed. For a failure, describe the failure scenario and its severity (critical, high, medium or low). Include severity in the text; the answer JSON format is unchanged.
- `repro_steps`: the steps you ran, in a `publishable` answer when your owner agrees, so the maintainer can rerun them.
- `evidence`: output, errors and logs from the run.

Skip criteria you couldn't test; a partial review is still useful.

The report must cover every criterion. Criteria without an authorized assessment
are marked `not_assessed`, rather than treated as passing. Missing verdicts or
missing severity on failures trigger regeneration, then withholding if the
report remains incomplete.
