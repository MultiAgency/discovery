# Releases

[`releases.json`](../releases.json) lists the releases respondents may trust:
network, OutLayer project, release (wasm sha256) and organizer account. The
survey Actions read the entry marked `"use": "surveys"`. There is currently no
active creation entry: the historical testnet hash is retained for existing
surveys while the scoped MVP awaits a newly approved release.

The current CLI and module use the reduced issue-survey report format. They do
not decode the removed review, task or wallet fields. Use the earlier CLI source
at commit `bdd9c6b` when retrieving or finalizing reports from the historical
release. Its immutable deployment and registered hash have not changed. Do not
route historical surveys through the current scheduled finalizer.

Respondents pin these values from `releases.json`, never from an
announcement: an announcement whose project, release or organizer doesn't
match an entry is refused.

Every entry is **locked**: its project's owner had no access keys on-chain
when the release was cut, so nobody can add a version that reads responses.
`discovery verify` checks this as of each survey's creation; local copies of a
deleted key don't matter.

Development runs on `spike1.research.agency.testnet/discovery-dev`, which is
unlocked and changes often. It is for synthetic data only, with
`--allow-unlocked`, and is deliberately not listed in `releases.json`.
