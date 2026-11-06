#!/usr/bin/env bash
# Local request-to-report demo. No credentials, no network beyond crate downloads.
# A local simulation: storage, block time and inference are simulated, so it
# is neither chain-verified nor evidence of deployed confidentiality.
set -euo pipefail
cargo test --quiet --locked --manifest-path "$(dirname "$0")/../Cargo.toml" \
  -p discovery-module demo::local_demo -- --exact --nocapture
