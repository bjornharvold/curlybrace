#!/usr/bin/env bash
# Policy guard: GitHub must never run database-backed tests.
#
# Scans every file under .github/workflows and .github/actions (workflows AND composite actions),
# case-insensitively, and FAILS on anything that could start or reach a database or run the local
# integration tiers: the integration gates, the runner scripts, docker run/compose, testcontainers,
# GitHub `services:` containers, MongoDB images/installs/binaries and ignored-test flags.
#
# Other files in .github (issue/PR templates, dependabot.yml) are documentation or config that
# cannot run anything, so they are deliberately out of scope (they legitimately mention MongoDB).
#
# Usage: scripts/check-no-db-in-ci.sh [repo-root]   (default: the repository containing this script)
# This script lives in scripts/, outside the scanned directories, so it does not match itself.
set -euo pipefail

root="${1:-$(cd "$(dirname "$0")/.." && pwd)}"

patterns='MONGOSTUDIO_IT|container-tests|integration-tests|docker[[:space:]]+(run|compose)|docker-compose|testcontainers|^[[:space:]-]*services[[:space:]]*:|mongo:|mongo-?db|mongod([^[:alnum:]]|$)|mongosh|brew[[:space:]]+install[[:space:]]+mongo'

dirs=()
for d in .github/workflows .github/actions; do
  [ -d "$root/$d" ] && dirs+=("$root/$d")
done
if [ "${#dirs[@]}" -eq 0 ]; then
  echo "no .github/workflows or .github/actions under $root: nothing to scan" >&2
  exit 2
fi

hits="$(grep -rniE "$patterns" "${dirs[@]}" || true)"

# Ignored tests are opt-in tiers, so `--include-ignored` / `--ignored` are forbidden too, with ONE
# exception: the database-free screenshot job, which must be exactly `--test snapshots` (a single
# named test target, no --workspace/--all) so it cannot widen into the database tiers.
ignored="$(grep -rniE -e '--include-ignored|--ignored' "${dirs[@]}" || true)"
if [ -n "$ignored" ]; then
  ignored="$(printf '%s\n' "$ignored" | awk '
    { n = gsub(/--test /, "&"); wide = ($0 ~ /--workspace|--all/) }
    !($0 ~ /--test snapshots/ && n == 1 && !wide)')"
fi

if [ -n "$hits" ] || [ -n "$ignored" ]; then
  [ -n "$hits" ] && printf '%s\n' "$hits"
  [ -n "$ignored" ] && printf '%s\n' "$ignored"
  echo "database-backed tests must not run on GitHub (see docs/superpowers/notes/testing.md)" >&2
  exit 1
fi
echo "ok: no workflow or action references the database-backed test gates"
