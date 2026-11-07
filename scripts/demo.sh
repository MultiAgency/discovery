#!/usr/bin/env bash
# Local request-to-report demo. No credentials, no network beyond crate downloads.
# A local simulation: approval, storage, block time and inference are simulated,
# so it is neither chain-verified nor evidence of deployed confidentiality.
#
#   scripts/demo.sh            print the transcript
#   scripts/demo.sh OUT_DIR    also save transcript.txt, released.md and
#                              withheld.md in OUT_DIR (new or empty)
set -euo pipefail
case $# in
  0) unset DISCOVERY_DEMO_OUT ;;
  1)
    mkdir -p -- "$1"
    [ -z "$(ls -A -- "$1")" ] || { echo "demo: $1 is not empty" >&2; exit 1; }
    DISCOVERY_DEMO_OUT="$(cd -- "$1" && pwd)"
    export DISCOVERY_DEMO_OUT
    ;;
  *) echo "usage: scripts/demo.sh [OUT_DIR]" >&2; exit 1 ;;
esac
cargo test --quiet --locked --manifest-path "$(dirname "$0")/../Cargo.toml" \
  -p discovery-module demo::local_demo -- --exact --nocapture
if [ $# -eq 1 ]; then echo "Saved transcript.txt, released.md and withheld.md in $1"; fi
