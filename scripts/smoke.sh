#!/usr/bin/env bash
# Synthetic testnet smoke test for the release registered in releases.json.
# Every signing step is a paid call from the organizer account. Run only with
# invented answers, after the release owner is locked: `discovery verify` and
# `discovery report` refuse unlocked releases and surveys created before the lock.
#
#   scripts/smoke.sh create NAME [MINUTES]          new survey on TARGET, verified
#   scripts/smoke.sh early NAME                     finalize before the deadline (must fail)
#   scripts/smoke.sh submit NAME ANSWERS.json       encrypted submission (DISCOVERY_PAYMENT_KEY)
#   scripts/smoke.sh finalize NAME                  finalize, fetch the verified report, check it
#   scripts/smoke.sh refinalize NAME                finalize again; identical output, verified report
#   scripts/smoke.sh check NAME                     closed, key gone, submissions refused
#   scripts/smoke.sh status NAME LABEL              reconcile a recorded transaction
#   scripts/smoke.sh workload COUNT DIR             COUNT distinct maximum-size answer files
#
# Each signed transaction is saved and its hash recorded in NAME/transactions.log
# before broadcasting. Re-running a step whose outcome is unknown re-broadcasts
# the same signed transaction instead of signing a replacement.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SYNTHETIC="$REPO/scripts/synthetic"
RELEASES="${RELEASES:-$REPO/releases.json}"
WORK="${DISCOVERY_SMOKE_DIR:-$HOME/.discovery-smoke}"
TARGET="${DISCOVERY_SMOKE_TARGET:-MultiAgency/discovery#1}"
CONTRACT=outlayer.testnet
RPC=https://archival-rpc.testnet.near.org
SECOND=1000000000

die() { printf 'smoke: %s\n' "$*" >&2; exit 1; }
ok() { printf '  ok  %s\n' "$*"; }

need() {
  local tool
  for tool in "$@"; do command -v "$tool" >/dev/null 2>&1 || die "missing $tool"; done
}

organizer() {
  local org
  org=$(jq -r '.service.organizer // empty' "$RELEASES")
  [[ -n "$org" ]] || die "releases.json registers no service"
  printf '%s' "$org"
}

# check_cli: before any paid step, the installed CLI must pin the release that
# releases.json registers. A stale install would sign for an organizer whose
# surveys `verify` then refuses. `prepare finalize` reports the pins offline.
check_cli() {
  local probe
  probe=$(discovery prepare finalize "$(printf '0%.0s' {1..64})" 2>/dev/null) ||
    die "the installed discovery CLI registers no release; reinstall it from this checkout"
  jq -e --slurpfile r "$RELEASES" '
      .source.Project.project_id == $r[0].service.project
      and .source.Project.version_key == $r[0].service.release
      and .secrets_ref.account_id == $r[0].service.organizer' <<<"$probe" >/dev/null ||
    die "the installed discovery CLI pins a different release than releases.json; reinstall it:
  cargo install --locked --path $REPO/crates/cli"
}

survey_dir() {
  [[ "${1:-}" =~ ^[a-z0-9-]+$ ]] || die "NAME must be lowercase letters, digits and dashes"
  printf '%s/%s' "$WORK" "$1"
}

# tx_output HASH FAILURE_FILE: output JSON, or record a confirmed terminal failure.
tx_output() {
  local reply
  reply=$(curl -fsS "$RPC" -H 'content-type: application/json' \
    -d "$(jq -nc --arg h "$1" --arg s "$(organizer)" \
      '{jsonrpc: "2.0", id: "smoke", method: "tx",
        params: {tx_hash: $h, sender_account_id: $s, wait_until: "FINAL"}}')") || return 1
  # An RPC error, timeout or non-final failure must remain pending. Only a
  # fully final chain failure permits signing another paid transaction.
  if jq -e '.result.final_execution_status == "FINAL" and
      (.result.status.Failure | type == "object")' <<<"$reply" >/dev/null; then
    jq --arg hash "$1" '{tx_hash: $hash, final_execution_status: .result.final_execution_status,
      status: .result.status}' <<<"$reply" > "$2.tmp"
    mv "$2.tmp" "$2"
    return 1
  fi
  if ! jq -e '.result.status.SuccessValue' <<<"$reply" >/dev/null; then
    printf '  %s: %s\n' "$1" "$(jq -c '.result.status // .error' <<<"$reply")" >&2
    return 1
  fi
  jq '.result.status.SuccessValue | @base64d | fromjson' <<<"$reply"
}

# save_output HASH FILE: write FILE only for a successful final outcome, so a
# failed or interrupted reconciliation never looks like a completed step.
save_output() {
  if tx_output "$1" "${2%.output.json}.failure.json" > "$2.tmp" && has_output "$2.tmp"; then
    mv "$2.tmp" "$2"
  else
    rm -f "$2.tmp"
    return 1
  fi
}

has_failure() {
  jq -e '.final_execution_status == "FINAL" and (.tx_hash | type == "string" and length > 0)
    and (.status.Failure | type == "object")' "$1" >/dev/null 2>&1
}

has_output() { jq -e 'type == "object"' "$1" >/dev/null 2>&1; }

# sign_and_send DIR LABEL REQUEST DEPOSIT: prints the transaction hash.
sign_and_send() {
  local dir="$1" label="$2" request="$3" deposit="$4" signed hash
  signed="$dir/$label.signed.json"
  if [[ -e "$signed" ]] && has_failure "$dir/$label.failure.json"; then
    # Fixed-label steps (init/early) also need a fresh transaction after a
    # terminal failure. Preserve both the signed bytes and failure evidence.
    hash=$(jq -r .tx_hash "$dir/$label.failure.json")
    mv "$signed" "$signed.failed-$hash"
    mv "$dir/$label.failure.json" "$dir/$label.failure.json.failed-$hash"
  fi
  if [[ -e "$signed" ]]; then
    printf '  %s was signed earlier; re-broadcasting the same transaction\n' "$label" >&2
  else
    near contract call-function as-transaction "$CONTRACT" request_execution \
      file-args "$request" prepaid-gas '300 Tgas' attached-deposit "$deposit" \
      sign-as "$(organizer)" network-config testnet sign-with-legacy-keychain \
      save-to-file "$signed" >&2
  fi
  hash=$(near --quiet transaction print-transaction signed \
      "$(jq -r .signed_transaction_as_base64 "$signed")" </dev/null 2>/dev/null |
    sed -n 's/^unsigned transaction hash (Base58-encoded SHA-256 hash): //p')
  [[ -n "$hash" ]] || die "could not read the hash of $signed"
  grep -qs "^$label $hash\$" "$dir/transactions.log" ||
    printf '%s %s\n' "$label" "$hash" >> "$dir/transactions.log"
  near transaction send-signed-transaction file-with-base64-signed-transaction "$signed" \
    network-config testnet send >&2 ||
    printf '  broadcast did not confirm; reconciling %s on-chain\n' "$hash" >&2
  if ! save_output "$hash" "$dir/$label.output.json"; then
    if has_failure "$dir/$label.failure.json"; then
      die "terminal on-chain failure for $label ($hash); see $dir/$label.failure.json.
Fix the cause, then rerun the step to sign a fresh transaction."
    fi
    die "no successful final outcome for $label ($hash). Reconcile with:
  scripts/smoke.sh status $(basename "$dir") $label
Only if it is unknown on-chain and was signed over a day ago (expired), delete
$signed to sign a new one."
  fi
  printf '%s' "$hash"
}

# Labels finalize-1, finalize-2, ...: reuse one whose outcome is unknown.
next_finalize_label() {
  local dir="$1" k=1
  while [[ -e "$dir/finalize-$k.signed.json" ]]; do
    has_output "$dir/finalize-$k.output.json" || has_failure "$dir/finalize-$k.failure.json" || break
    k=$((k + 1))
  done
  printf 'finalize-%s' "$k"
}

# Public-report checks that need no knowledge of private answers beyond the fixtures.
check_public() {
  local file="$1" failed=0
  # The uppercase marker starts only context-only fixture text, which must
  # never appear verbatim; publishable fixtures may be quoted and carry none.
  if grep -Eq 'SYNTHETIC' "$file" ||
    grep -Eiq 'example\.invalid|/home/|team-lead|88231947|10\.20\.30\.40|qa-runner|build-farm|synthetic-(project|corp)|payments team' "$file"; then
    printf '  FAIL fixture text or identifier in %s\n' "$file"; failed=1
  fi
  if grep -Eq '\bS[0-9]+\.[0-9]+\b' "$file"; then
    printf '  FAIL private source label in %s\n' "$file"; failed=1
  fi
  # The module's publication check (crates/module/src/checks.rs) uses the same
  # rule: a quantity word, at most two words (three after "of") before a word
  # for respondents.
  # Each paragraph is joined into one line, so a count split across a line
  # break is still caught, but template text (such as the model name) never
  # pairs with the next paragraph.
  if awk 'BEGIN { RS = "" } { gsub(/\n/, " "); print }' "$file" | grep -Eiq '(^|[^[:alnum:]])([0-9]+|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|dozen|both|single|couple|handful|few|several|many|most|half|majority|minority|none|neither)([^[:alnum:]]+of([^[:alnum:]]+[[:alnum:]]+){0,2}|([^[:alnum:]]+[[:alnum:]]+){0,2})[^[:alnum:]]+((respondent|responder|response|participant|person|agent|submission)s?|people)([^[:alnum:]]|$)'; then
    printf '  FAIL respondent count in %s\n' "$file"; failed=1
  fi
  if grep -Eiq 'reuses confidential|confidential identifier|audit:' "$file"; then
    printf '  FAIL rejection detail in %s\n' "$file"; failed=1
  fi
  return "$failed"
}

# fetch_report DIR HASH OUT: the CLI verifies HASH against the pinned release
# and the owner's lock at that block before printing; then the public checks.
fetch_report() {
  discovery --target "$TARGET" report "$(cat "$1/survey_id")" --finalize-tx "$2" > "$3" ||
    die "finalization $2 did not verify; see the error above"
  ok "report verified on-chain ($2): $3"
  check_public "$3" || return 1
  ok "no fixture text, identifiers, source labels, counts or rejection details"
}

# prepare_init DIR MINUTES TITLE: definition.json with a deadline MINUTES from
# now, on TARGET, then the unsigned creation request.
prepare_init() {
  local dir="$1" survey deadline
  [[ "$2" =~ ^[0-9]+$ ]] || die "MINUTES must be a whole number"
  mkdir -p "$dir"
  deadline=$(( $(date +%s) + $2 * 60 ))
  # jq would round a nanosecond timestamp through a double, so substitute it as text.
  jq --arg t "$3" --arg target "$TARGET" '.title = $t
      | .target = {repo: ($target | split("#")[0]), number: ($target | split("#")[1] | tonumber)}' \
    "$SYNTHETIC/definition.json" |
    sed "s/\"deadline_ns\": 0,/\"deadline_ns\": ${deadline}000000000,/" > "$dir/definition.json"
  grep -q "\"deadline_ns\": ${deadline}000000000," "$dir/definition.json" || die "failed to set deadline_ns"
  discovery --target "$TARGET" prepare init "$dir/definition.json" \
    > "$dir/init-request.json" 2> "$dir/prepare.log" || { cat "$dir/prepare.log" >&2; rm -f "$dir/init-request.json"; exit 1; }
  survey=$(sed -n 's/^survey_id: //p' "$dir/prepare.log")
  printf '%s\n' "$survey" > "$dir/survey_id"
}

cmd_create() {
  local name="${1:?NAME}" minutes="${2:-15}" dir survey hash
  need discovery near jq curl
  check_cli
  dir=$(survey_dir "$name")
  if [[ -e "$dir/init-request.json" ]]; then
    printf '  resuming %s; use a new NAME for a new survey\n' "$name"
  else
    prepare_init "$dir" "$minutes" "Smoke test $name"
  fi
  survey=$(cat "$dir/survey_id")
  hash=$(sign_and_send "$dir" init "$dir/init-request.json" '0.1 NEAR')
  printf '%s\n' "$hash" > "$dir/init_tx"
  jq -e --arg s "$survey" '.survey_id == $s and (has("error") | not)' "$dir/init.output.json" >/dev/null ||
    die "creation output: $(jq -c . "$dir/init.output.json")"
  ok "created $survey in $hash"
  discovery --target "$TARGET" verify "$survey" --init-tx "$hash" > "$dir/verify-open.json"
  jq -e '.accepting_responses and (.closed | not) and .key_present' "$dir/verify-open.json" >/dev/null ||
    die "survey does not accept responses: $(jq -c . "$dir/verify-open.json")"
  ok "verified against the locked release; accepting responses"
  printf '\nrelease: %s\nsurvey: %s\ninit_tx: %s\ndeadline: %s\n' \
    "$(jq -r .release "$dir/verify-open.json")" "$survey" "$hash" \
    "$(jq -r '.deadline_ns / 1e9 | floor | todate' "$dir/definition.json")"
}

cmd_early() {
  local dir hash
  need discovery near jq curl
  check_cli
  dir=$(survey_dir "${1:?NAME}")
  discovery prepare finalize "$(cat "$dir/survey_id")" > "$dir/early-request.json"
  hash=$(sign_and_send "$dir" early "$dir/early-request.json" '0.2 NEAR')
  jq -e '.error == "the collection period has not ended"' "$dir/early.output.json" >/dev/null ||
    die "early finalization in $hash returned $(jq -c . "$dir/early.output.json")"
  ok "early finalization refused in $hash"
}

cmd_submit() {
  local dir
  need discovery
  dir=$(survey_dir "${1:?NAME}")
  [[ -f "${2:-}" ]] || die "usage: submit NAME ANSWERS.json"
  discovery --target "$TARGET" submit "$(cat "$dir/survey_id")" --init-tx "$(cat "$dir/init_tx")" "$2"
  printf '%s %s\n' "$(date -u +%FT%TZ)" "$(basename "$2")" >> "$dir/submissions.log"
}

cmd_finalize() {
  local dir survey label hash deadline
  need discovery near jq curl
  check_cli
  dir=$(survey_dir "${1:?NAME}")
  survey=$(cat "$dir/survey_id")
  deadline=$(jq -r ".deadline_ns / $SECOND | floor" "$dir/definition.json")
  # Block time can trail the local clock.
  (( $(date +%s) >= deadline + 60 )) || die "wait until $(( deadline + 60 - $(date +%s) )) s from now"
  label=$(next_finalize_label "$dir")
  if [[ ! -e "$dir/$label.signed.json" ]]; then
    discovery prepare finalize "$survey" > "$dir/$label-request.json"
  fi
  hash=$(sign_and_send "$dir" "$label" "$dir/$label-request.json" '0.2 NEAR')
  if jq -e 'has("error")' "$dir/$label.output.json" >/dev/null; then
    die "finalization did not complete in $hash: $(jq -r .error "$dir/$label.output.json"); run finalize again"
  fi
  printf '%s\n' "$hash" > "$dir/finalize_tx"
  printf '  status: %s\n' "$(jq -r .status "$dir/$label.output.json")"
  fetch_report "$dir" "$hash" "$dir/report.md"
}

cmd_refinalize() {
  local dir label hash
  need discovery near jq curl
  check_cli
  dir=$(survey_dir "${1:?NAME}")
  [[ -s "$dir/finalize_tx" ]] || die "finalize first"
  label=$(next_finalize_label "$dir")
  [[ -e "$dir/$label.signed.json" ]] ||
    discovery prepare finalize "$(cat "$dir/survey_id")" > "$dir/$label-request.json"
  hash=$(sign_and_send "$dir" "$label" "$dir/$label-request.json" '0.2 NEAR')
  local first
  first=$(grep "^finalize-[0-9]* $(cat "$dir/finalize_tx")\$" "$dir/transactions.log" | cut -d' ' -f1)
  jq -e --slurpfile a "$dir/$first.output.json" '. == $a[0]' "$dir/$label.output.json" >/dev/null ||
    die "finalization retry $hash returned a different output"
  ok "retry $hash returned the identical stored report"
  fetch_report "$dir" "$hash" "$dir/report-$hash.md"
  printf '  Its OutLayer job should be short: a stored report needs no inference.\n'
}

cmd_check() {
  local dir survey out
  need discovery jq
  dir=$(survey_dir "${1:?NAME}")
  survey=$(cat "$dir/survey_id")
  discovery --target "$TARGET" verify "$survey" --init-tx "$(cat "$dir/init_tx")" > "$dir/verify-closed.json"
  jq -e '.closed and (.key_present | not) and .report_released and (.accepting_responses | not)' \
    "$dir/verify-closed.json" >/dev/null || die "not cleaned up: $(jq -c . "$dir/verify-closed.json")"
  ok "closed, key replaced, report stored"
  if out=$(discovery --target "$TARGET" submit "$survey" --init-tx "$(cat "$dir/init_tx")" \
      "$SYNTHETIC/answers1.json" 2>&1); then
    die "a submission after finalization was accepted"
  fi
  [[ "$out" == *"survey is closed"* ]] || die "unexpected refusal: $out"
  ok "further submissions refused: survey is closed"
}

cmd_status() {
  local dir hash
  need near jq curl
  dir=$(survey_dir "${1:?NAME}")
  hash=$(grep "^${2:?LABEL} " "$dir/transactions.log" | tail -n1 | cut -d' ' -f2)
  [[ -n "$hash" ]] || die "no transaction recorded for $2"
  near transaction view-status "$hash" network-config testnet ||
    printf '  CLI status lookup failed; reconciling %s through RPC\n' "$hash" >&2
  if ! save_output "$hash" "$dir/$2.output.json"; then
    if has_failure "$dir/$2.failure.json"; then
      die "terminal on-chain failure for $2 ($hash); fix the cause, then rerun the step"
    fi
    die "no successful final outcome for $2 ($hash)"
  fi
  cat "$dir/$2.output.json"
}

cmd_workload() {
  local count="${1:?COUNT}" out="${2:?DIR}" i
  need jq
  [[ "$count" =~ ^[0-9]+$ ]] || die "COUNT must be a number"
  mkdir -p "$out"
  for (( i = 1; i <= count; i++ )); do
    jq -c --argjson i "$i" '
      def fill($n): [range(0; $n) | "Synthetic workload sentence \($i)-\(.) about setup, verification and disclosure."] | join(" ");
      {answers: [
        {question_id: "workflow", basis: "observed", disclosure: {tier: "publishable"},
         text: ("Synthetic workload \($i): " + fill(16))},
        {question_id: "obstacle", basis: "interpretation", disclosure: {tier: "context_only", phrasing: "a synthetic workload respondent"},
         text: ("SYNTHETIC workload \($i): " + fill(16))},
        {question_id: "outcome", basis: "confirmed", disclosure: {tier: "context_only"},
         text: ("SYNTHETIC workload \($i): " + fill(16))}]}' -n > "$out/workload-$i.json"
    (( $(wc -c < "$out/workload-$i.json") <= 4092 )) || die "workload-$i.json exceeds 4092 bytes"
  done
  ok "$count files in $out, $(wc -c < "$out/workload-1.json") bytes each (limit 4092)"
}

main() {
  local command="${1:-}"
  shift || true
  case "$command" in
    create | early | submit | finalize | refinalize | check | status | workload) "cmd_$command" "$@" ;;
    *) sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
  esac
}

# Sourcing defines the functions without running anything (scripts/test-smoke.sh).
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
