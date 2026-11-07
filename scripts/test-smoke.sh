#!/usr/bin/env bash
# Offline regression tests for smoke.sh transaction recovery. Stubs `near` and
# `curl`; nothing reaches the network or signs anything.
#
#   scripts/test-smoke.sh
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin"

# near: print-transaction reports a fixed hash; broadcasting always fails.
cat > "$tmp/bin/near" <<'EOF'
#!/bin/sh
case "$*" in
  *save-to-file*)
    for arg do signed="$arg"; done
    printf '{"signed_transaction_as_base64":"BBBB"}\n' > "$signed"
    printf 'signed\n' >> "$STUB_SIGN_LOG"
    ;;
  *print-transaction*) echo "unsigned transaction hash (Base58-encoded SHA-256 hash): HashOfTheSignedTx1111111111111111111111" ;;
  *send-signed-transaction*) exit 1 ;;
  *view-status*) exit 1 ;;
  *) echo "unexpected near call: $*" >&2; exit 99 ;;
esac
EOF
# curl: replies with $STUB_REPLY, or fails like a timeout when it is empty.
cat > "$tmp/bin/curl" <<'EOF'
#!/bin/sh
[ -n "$STUB_REPLY" ] || exit 28
printf '%s' "$STUB_REPLY"
EOF
chmod +x "$tmp/bin/near" "$tmp/bin/curl"
export PATH="$tmp/bin:$PATH" DISCOVERY_SMOKE_DIR="$tmp/work" STUB_SIGN_LOG="$tmp/sign.log"

# shellcheck source=scripts/smoke.sh
. "$repo/scripts/smoke.sh"
# shellcheck disable=SC2329 # replaces smoke.sh's, called indirectly
organizer() { printf 'organizer.testnet'; }

failures=0
# shellcheck disable=SC2329 # called through expect
fails() { ! ( "$@" ) >/dev/null 2>&1; }
# shellcheck disable=SC2329 # called through expect
succeeds() { ( "$@" ) >/dev/null 2>&1; }
expect() { # expect DESCRIPTION COMMAND...
  local what="$1"; shift
  if "$@"; then printf '  ok    %s\n' "$what"; else printf '  FAIL  %s\n' "$what"; failures=1; fi
}

dir="$tmp/work/survey"
mkdir -p "$dir"
echo '{"signed_transaction_as_base64": "AAAA"}' > "$dir/finalize-1.signed.json"
success=$(jq -nc '{result: {status: {SuccessValue: ({survey_id: "s", status: "released"} | tojson | @base64)}}}')
failure='{"result": {"final_execution_status": "FINAL", "status": {"Failure": {"ActionError": {}}}}}'
unknown='{"error": {"cause": {"name": "UNKNOWN_TRANSACTION"}}}'

nonfinal=$(jq '.result.final_execution_status = "EXECUTED_OPTIMISTIC"' <<<"$failure")
for reply in "" "$unknown" "$nonfinal"; do
  export STUB_REPLY="$reply"
  label="${reply:0:30}"
  # The recorded hash is needed for status; sign_and_send writes it.
  printf 'finalize-1 HashOfTheSignedTx1111111111111111111111\n' > "$dir/transactions.log"
  expect "status fails for reply '${label:-timeout}'" fails cmd_status survey finalize-1
  expect "no output file after reply '${label:-timeout}'" test ! -e "$dir/finalize-1.output.json"
  expect "finalize-1 is still pending" test "$(next_finalize_label "$dir")" = finalize-1
  ( sign_and_send "$dir" finalize-1 /dev/null '0.2 NEAR' >/dev/null 2>&1 ) || true
  expect "re-broadcast leaves no output file" test ! -e "$dir/finalize-1.output.json"
  expect "pending attempts never sign a replacement" test ! -e "$STUB_SIGN_LOG"
  expect "pending outcomes never record a terminal failure" test ! -e "$dir/finalize-1.failure.json"
done

export STUB_REPLY="$failure"
expect "status reports a terminal failure" fails cmd_status survey finalize-1
expect "a terminal failure permits the next attempt" test "$(next_finalize_label "$dir")" = finalize-2
expect "terminal failure is saved separately" has_failure "$dir/finalize-1.failure.json"
expect "terminal failure is not module output" test ! -e "$dir/finalize-1.output.json"
export STUB_REPLY="$success"
expect "finalization retry signs and saves the next attempt" succeeds sign_and_send "$dir" "$(next_finalize_label "$dir")" /dev/null '0.2 NEAR'
expect "new finalization has a successful output" has_output "$dir/finalize-2.output.json"
expect "original failure remains available" has_failure "$dir/finalize-1.failure.json"

# A fixed-label step must preserve the failed attempt and sign fresh bytes.
cp "$dir/finalize-1.signed.json" "$dir/init.signed.json"
cp "$dir/finalize-1.failure.json" "$dir/init.failure.json"
export STUB_REPLY="$success"
expect "fixed-label retry succeeds with a fresh transaction" succeeds sign_and_send "$dir" init /dev/null '0.1 NEAR'
expect "each retry signed exactly once" test "$(wc -l < "$STUB_SIGN_LOG" | tr -d ' ')" = 2
expect "fixed-label retry preserves signed failure evidence" test -e "$dir/init.signed.json.failed-HashOfTheSignedTx1111111111111111111111"
expect "fixed-label retry preserves the failure record" has_failure "$dir/init.failure.json.failed-HashOfTheSignedTx1111111111111111111111"
expect "fixed-label retry stores the successful output" has_output "$dir/init.output.json"
rm -f "$dir/finalize-1.failure.json"

: > "$dir/finalize-1.output.json"
expect "an empty output file from an older run does not count" \
  test "$(next_finalize_label "$dir")" = finalize-1
echo 'not json' > "$dir/finalize-1.output.json"
expect "an invalid output file does not count" test "$(next_finalize_label "$dir")" = finalize-1
rm "$dir/finalize-1.output.json"

export STUB_REPLY="$success"
expect "status saves a successful outcome" succeeds cmd_status survey finalize-1
expect "the saved outcome is the module output" \
  jq -e '.survey_id == "s"' "$dir/finalize-1.output.json" >/dev/null
expect "the next finalization skips both completed attempts" test "$(next_finalize_label "$dir")" = finalize-3

# check_public: the uppercase fixture marker and identifiers fail; ordinary
# words such as an approved phrasing ("a synthetic workload respondent") pass.
report="$tmp/report.md"
printf 'SYNTHETIC: Setup should list every missing local setting.\n' > "$report"
expect "a verbatim fixture marker fails" fails check_public "$report"
printf 'Seen on qa-runner-3.internal.example.invalid.\n' > "$report"
expect "a fixture identifier fails" fails check_public "$report"
# The count phrases the module's tests use (crates/module/src/checks.rs).
phrases="$repo/scripts/synthetic/count-phrases.json"
# By index, so a phrase containing a line break stays one report.
for (( i = 0; i < $(jq '.counted | length' "$phrases"); i++ )); do
  jq -r ".counted[$i]" "$phrases" > "$report"
  expect "a respondent count fails: $(jq -c ".counted[$i]" "$phrases")" fails check_public "$report"
done
# A deployed report (survey 0215b5db…): the model name's "5.3" must not pair
# with "Respondents" in the next paragraph.
printf 'Survey 0215b5db25ca · model z-ai/glm-5.3-flash\n\nRespondents have not provided any surfaceable evidence.\n' > "$report"
expect "a count never spans paragraphs" succeeds check_public "$report"
while IFS= read -r uncounted; do
  printf '%s\n' "$uncounted" > "$report"
  expect "no respondent count passes: $uncounted" succeeds check_public "$report"
done < <(jq -r '.uncounted[]' "$phrases")

# check_cli: the installed CLI must pin the release in releases.json.
export RELEASES="$tmp/releases.json"
printf '{"service":{"project":"o.testnet/p","release":"%s","organizer":"org.testnet"}}' \
  "$(printf 'a%.0s' {1..64})" > "$RELEASES"
cat > "$tmp/bin/discovery" <<'EOF2'
#!/bin/sh
printf '{"source":{"Project":{"project_id":"o.testnet/p","version_key":"%s"}},"secrets_ref":{"account_id":"org.testnet"}}' "$STUB_RELEASE"
EOF2
chmod +x "$tmp/bin/discovery"
STUB_RELEASE=$(printf 'a%.0s' {1..64}) expect "a CLI pinning releases.json passes" succeeds check_cli
STUB_RELEASE=$(printf 'b%.0s' {1..64}) expect "a CLI pinning another release fails" fails check_cli

# fetch_report: a finalization that fails CLI verification stops the step; a
# verified report must also pass the public checks.
echo s > "$dir/survey_id"
cat > "$tmp/bin/discovery" <<'EOF2'
#!/bin/sh
[ -z "$STUB_VERIFY_FAILS" ] || { echo "owner was not locked" >&2; exit 1; }
printf '%s\n' "$STUB_REPORT"
EOF2
chmod +x "$tmp/bin/discovery"
STUB_REPORT='Setup should list every missing local setting.' \
  expect "a verified, clean report passes" succeeds fetch_report "$dir" HashOfTheSignedTx1111111111111111111111 "$tmp/r.md"
STUB_VERIFY_FAILS=1 \
  expect "an unverified finalization fails" fails fetch_report "$dir" HashOfTheSignedTx1111111111111111111111 "$tmp/r.md"
STUB_REPORT='Seen on qa-runner-3.internal.example.invalid.' \
  expect "a verified report with fixture identifiers fails" fails fetch_report "$dir" HashOfTheSignedTx1111111111111111111111 "$tmp/r.md"

# prepare_init: each smoke survey gets its own title, so its own survey id.
( prepare_init "$tmp/init-b" 15 "Smoke test b" ) >/dev/null 2>&1
expect "prepare_init sets the survey title" test "$(jq -r .title "$tmp/init-b/definition.json")" = "Smoke test b"

exit "$failures"
