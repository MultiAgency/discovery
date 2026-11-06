#!/bin/sh
# Usage: fresh-definition.sh OUT.json [MINUTES] [TITLE] [OWNER/REPO#NUMBER]
# Copies definition.json with a deadline MINUTES from now (default 15) and,
# optionally, another title (a new survey id) or target issue. Run it
# immediately before `discovery prepare init`: creation fails once the
# deadline has passed.
set -eu
out="${1:?usage: fresh-definition.sh OUT.json [MINUTES] [TITLE] [OWNER/REPO#NUMBER]}"
minutes="${2:-15}"
title="${3:-}"
target="${4:-}"
here=$(dirname "$0")
case "$minutes" in '' | *[!0-9]*) echo "MINUTES must be a whole number" >&2; exit 2 ;; esac
case "$target" in '' | */*'#'[1-9]*) ;; *) echo "target must be OWNER/REPO#NUMBER" >&2; exit 2 ;; esac
seconds=$(( $(date +%s) + minutes * 60 ))
# jq would round a nanosecond timestamp through a double, so substitute it as text.
jq --arg t "$title" --arg target "$target" '
  (if $t == "" then . else .title = $t end)
  | if $target == "" then . else
      .target = {repo: ($target | split("#")[0]), number: ($target | split("#")[1] | tonumber)}
    end' "$here/definition.json" |
  sed "s/\"deadline_ns\": 0,/\"deadline_ns\": ${seconds}000000000,/" > "$out"
grep -q "\"deadline_ns\": ${seconds}000000000," "$out" || { echo "failed to set deadline_ns" >&2; exit 1; }
echo "$out: deadline $(jq -nr --argjson s "$seconds" '$s | todate') (${minutes} min)"
