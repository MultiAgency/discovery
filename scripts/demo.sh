#!/usr/bin/env bash
# Local request-to-report demo. No credentials, no network beyond crate downloads.
#
#   scripts/demo.sh [OUT_DIR]     (default: target/demo)
#
# Runs the real survey, envelope, publication-check, finalization and cleanup
# code on scripts/synthetic/ with simulated storage, time and inference, then
# writes transcript.txt, outcomes.json, report.md and withheld.md to OUT_DIR.
# Exits non-zero if any outcome fails. Nothing here is chain-verified or
# evidence of deployed confidentiality.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="${1:-$repo/target/demo}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
rm -f "$out/transcript.txt" "$out/outcomes.json" "$out/report.md" "$out/withheld.md"

DISCOVERY_DEMO_DIR="$out" cargo test --quiet --locked --manifest-path "$repo/Cargo.toml" \
  -p discovery-module demo::local_demo -- --exact --nocapture

printf '\nArtifacts (local simulation, not chain-verified):\n'
for file in transcript.txt outcomes.json report.md withheld.md; do
  printf '  %s\n' "$out/$file"
done
