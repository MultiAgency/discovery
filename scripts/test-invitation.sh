#!/usr/bin/env bash
# Offline tests for invitation.sh's checkout checks. `cargo` is stubbed, so
# nothing is built, verified or paid for; rendering is tested in
# crates/cli/src/invitation.rs.
#
#   scripts/test-invitation.sh
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/work/scripts"
cat > "$tmp/bin/cargo" <<'STUB'
#!/bin/sh
printf '%s\n' "$@" > "$STUB_CARGO_ARGS"
STUB
chmod +x "$tmp/bin/cargo"
export PATH="$tmp/bin:$PATH" STUB_CARGO_ARGS="$tmp/cargo-args"
# Isolate from the user's git configuration, such as commit signing.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid
export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid

cd "$tmp/work"
cp "$repo/scripts/invitation.sh" scripts/
echo tracked > file
git init -q -b main . && git add . && git commit -qm one
git init -q --bare "$tmp/remote.git"
canonical=https://github.com/MultiAgency/discovery
git remote add origin "$canonical"
commit=$(git rev-parse HEAD)
survey=$(printf 'c%.0s' {1..64})
args=("$survey" CreationTx111111111111111111111111111111 MultiAgency/discovery#1)
page=https://example.invalid/invitation

failures=0
expect() { # expect DESCRIPTION COMMAND...
  local what="$1"; shift
  if "$@"; then printf '  ok    %s\n' "$what"; else printf '  FAIL  %s\n' "$what"; failures=1; fi
}
# shellcheck disable=SC2329 # called through expect
refused() { # refused MESSAGE ARGS...: fails, names MESSAGE, never runs cargo
  local message="$1"; shift
  rm -f "$STUB_CARGO_ARGS"
  ! scripts/invitation.sh "$@" 2>"$tmp/err" >/dev/null &&
    grep -q "$message" "$tmp/err" && [ ! -e "$STUB_CARGO_ARGS" ]
}

expect "missing values are refused" refused usage "${args[@]}" "$commit"
expect "an unpushed commit is refused" refused "no fetched origin" "${args[@]}" "$commit" "$page"
# Populate cached refs entirely locally; no test contacts GitHub.
git push -q "$tmp/remote.git" main
git fetch -q "$tmp/remote.git" main:refs/remotes/origin/main
git remote set-url origin "$tmp/remote.git"
expect "a noncanonical origin is refused despite a matching cached ref" refused "canonical MultiAgency/discovery" "${args[@]}" "$commit" "$page"
git remote set-url origin "$canonical"
git update-ref -d refs/remotes/origin/main
git update-ref refs/remotes/other/main "$commit"
expect "another remote's cached ref does not prove publication" refused "no fetched origin" "${args[@]}" "$commit" "$page"
git update-ref refs/remotes/origin/main "$commit"
expect "an abbreviated commit is refused" refused "not this checkout's HEAD" "${args[@]}" "${commit:0:12}" "$page"
echo changed > file
expect "tracked changes are refused" refused "tracked files differ" "${args[@]}" "$commit" "$page"
git commit -qam two
expect "a commit other than HEAD is refused" refused "not this checkout's HEAD" "${args[@]}" "$commit" "$page"
git push -q "$tmp/remote.git" main
git fetch -q "$tmp/remote.git" main:refs/remotes/origin/main
commit=$(git rev-parse HEAD)
echo untracked > untracked
rm -f "$STUB_CARGO_ARGS"
scripts/invitation.sh "${args[@]}" "$commit" "$page" >/dev/null
expected=(run --quiet --locked -p discovery-cli -- --target MultiAgency/discovery#1
  invitation "$survey" --init-tx "${args[1]}" --commit "$commit" --page "$page")
expect "a clean HEAD on canonical origin's cached branch runs the pinned CLI with exact values" \
  diff <(printf '%s\n' "${expected[@]}") "$STUB_CARGO_ARGS"

git remote set-url origin git@github.com:MultiAgency/discovery.git
expect "the canonical SSH origin is accepted" scripts/invitation.sh "${args[@]}" "$commit" "$page"

[ "$failures" -eq 0 ] && echo "all invitation checks passed"
exit "$failures"
