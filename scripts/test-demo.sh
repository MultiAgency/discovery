#!/usr/bin/env bash
# Offline checks for the demo launcher; cargo is stubbed, so no build or
# network access occurs. Rust tests cover the generated artifact contents.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/work"
cat > "$tmp/bin/cargo" <<'STUB'
#!/bin/sh
printf '%s' "${DISCOVERY_DEMO_OUT-unset}" > "$STUB_DEMO_ENV"
exit "${STUB_CARGO_EXIT:-0}"
STUB
chmod +x "$tmp/bin/cargo"
export PATH="$tmp/bin:$PATH" STUB_DEMO_ENV="$tmp/env"
cd "$tmp/work"

DISCOVERY_DEMO_OUT="$tmp/unrequested" "$repo/scripts/demo.sh" > "$tmp/out"
[ "$(cat "$STUB_DEMO_ENV")" = unset ]
[ ! -e "$tmp/unrequested" ]
echo '  ok    no-argument run clears inherited artifact destination'

"$repo/scripts/demo.sh" -output > "$tmp/out"
[ "$(cat "$STUB_DEMO_ENV")" = "$tmp/work/-output" ]
echo '  ok    explicit output paths beginning with a dash are literal paths'

echo keep > -output/existing
rm -f "$STUB_DEMO_ENV"
if "$repo/scripts/demo.sh" -output > "$tmp/out" 2> "$tmp/err"; then
  echo 'FAIL: nonempty output directory accepted' >&2
  exit 1
fi
[ ! -e "$STUB_DEMO_ENV" ]
[ "$(cat -- -output/existing)" = keep ]
echo '  ok    nonempty output directory is refused before cargo and preserved'

if STUB_CARGO_EXIT=7 "$repo/scripts/demo.sh" "$tmp/failed" > "$tmp/out" 2> "$tmp/err"; then
  echo 'FAIL: cargo failure accepted' >&2
  exit 1
fi
if grep -q Saved "$tmp/out"; then
  echo 'FAIL: cargo failure announced saved artifacts' >&2
  exit 1
fi
echo '  ok    cargo failure does not announce saved artifacts'
echo 'all demo launcher checks passed'
