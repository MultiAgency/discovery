#!/usr/bin/env bash
# Render the participant invitation for a live survey from this checkout.
#
#   scripts/invitation.sh SURVEY INIT_TX OWNER/REPO#NUMBER COMMIT PAGE_URL > invitation.md
#
# Needs DISCOVERY_PAYMENT_KEY, the published submission key: verification makes
# one paid status call. COMMIT must be this checkout's HEAD, without tracked
# changes, and reachable from a cached origin branch of the canonical GitHub
# repository, so respondents can install this commit. Fetch origin before use:
# this local check does not establish that the remote still contains it.
# The page is printed only after `discovery`
# verifies the survey in this run and finds it accepting responses.
set -euo pipefail

die() { printf 'invitation: %s\n' "$*" >&2; exit 1; }
[ $# -eq 5 ] || die "usage: scripts/invitation.sh SURVEY INIT_TX OWNER/REPO#NUMBER COMMIT PAGE_URL"
survey=$1 init_tx=$2 target=$3 commit=$4 page=$5

cd "$(dirname "${BASH_SOURCE[0]}")/.."
head=$(git rev-parse HEAD)
[ "$commit" = "$head" ] || die "COMMIT is not this checkout's HEAD ($head)"
[ -z "$(git status --porcelain --untracked-files=no)" ] ||
  die "tracked files differ from COMMIT; the CLI would not match what respondents install"
origin=$(git remote get-url origin 2>/dev/null) || die "origin must be the canonical MultiAgency/discovery repository"
case "$origin" in
  https://github.com/MultiAgency/discovery|https://github.com/MultiAgency/discovery.git|git@github.com:MultiAgency/discovery|git@github.com:MultiAgency/discovery.git) ;;
  *) die "origin must be the canonical MultiAgency/discovery repository" ;;
esac
[ -n "$(git for-each-ref --format='%(refname)' --contains="$commit" refs/remotes/origin/)" ] ||
  die "COMMIT is on no fetched origin branch; push it to the canonical repository, fetch origin, and retry"

exec cargo run --quiet --locked -p discovery-cli -- \
  --target "$target" invitation "$survey" --init-tx "$init_tx" --commit "$commit" --page "$page"
