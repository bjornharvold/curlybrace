#!/usr/bin/env bash
# Tests for scripts/check-no-db-in-ci.sh: runs the guard against fixture trees and asserts exit
# codes (clean passes; every forbidden pattern, in workflows and in composite actions, fails).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
guard="$here/check-no-db-in-ci.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
failures=0

expect() { # expect <exit code> <description> <root>
  local want="$1" what="$2" root="$3" got=0
  "$guard" "$root" >/dev/null 2>&1 || got=$?
  if [ "$got" -ne "$want" ]; then
    echo "FAIL: $what (expected exit $want, got $got)" >&2
    failures=$((failures + 1))
  else
    echo "ok:   $what"
  fi
}

fixture() { # fixture <name> <relative file> <line>  -> prints the fixture root
  local root="$work/$1"
  mkdir -p "$root/$(dirname "$2")"
  printf 'name: x\njobs:\n  j:\n    steps:\n      - run: %s\n' "$3" >"$root/$2"
  echo "$root"
}

expect 0 "clean workflow passes" "$(fixture clean .github/workflows/ci.yml 'cargo test --workspace')"

# Docs/config outside workflows and actions may mention MongoDB.
mkdir -p "$work/docs/.github/ISSUE_TEMPLATE" "$work/docs/.github/workflows"
echo "MongoDB server version? CURLYBRACE_IT=1" >"$work/docs/.github/ISSUE_TEMPLATE/bug.md"
echo "name: ok" >"$work/docs/.github/workflows/ci.yml"
expect 0 "templates outside workflows/actions are out of scope" "$work/docs"

i=0
while IFS= read -r line; do
  i=$((i + 1))
  expect 1 "workflow forbids: $line" "$(fixture "w$i" .github/workflows/ci.yml "$line")"
done <<'LINES'
CURLYBRACE_IT=1 cargo test
curlyBRACE_it=1 cargo test
scripts/integration-tests.sh
scripts/container-tests.sh
docker run -d mongo:7
docker   compose up
docker-compose up
cargo add testcontainers
brew install mongodb-community@7.0
mongod --dbpath /tmp/x
mongosh --eval 1
cargo test -- --include-ignored
cargo test -- --ignored
cargo test --workspace --locked -- --include-ignored
cargo test -p curlybrace-ui --test snapshots --test local_e2e_flows -- --include-ignored
cargo test --test snapshots --workspace -- --include-ignored
setup-mongodb
LINES

expect 0 "the DB-free screenshot job may use --include-ignored" \
  "$(fixture snap .github/workflows/ci.yml 'cargo test -p curlybrace-ui --test snapshots --locked -- --include-ignored')"

mkdir -p "$work/svc/.github/workflows"
printf 'jobs:\n  j:\n    services:\n      db:\n        image: postgres\n' >"$work/svc/.github/workflows/ci.yml"
expect 1 "workflow forbids services: containers" "$work/svc"

expect 1 "composite action is scanned" \
  "$(fixture act .github/actions/setup/action.yml 'scripts/integration-tests.sh')"

mkdir -p "$work/empty"
expect 2 "no .github dirs is an error, not a silent pass" "$work/empty"

echo "real tree:"
if "$guard" >/dev/null; then echo "ok:   current repository is clean"; else
  echo "FAIL: current repository tree" >&2
  failures=$((failures + 1))
fi

if [ "$failures" -ne 0 ]; then
  echo "$failures guard test(s) failed" >&2
  exit 1
fi
echo "all guard tests passed"
