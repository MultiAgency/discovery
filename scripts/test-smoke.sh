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

exit "$failures"
