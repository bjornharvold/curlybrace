#!/usr/bin/env bash
# Tier 2 "docker" tests: create their OWN throwaway containers (names ms-it-*) for the cases that
# need control of the server: --auth, stopping and restarting mid-session, the SSH tunnel stack.
# Never touches containers it did not create. Manual only; never run on GitHub.
#
#   scripts/container-tests.sh
#   scripts/container-tests.sh -- --test-threads=2     # fewer parallel containers
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v docker >/dev/null 2>&1 || ! docker info >/dev/null 2>&1; then
  echo "docker is not available (is the daemon running?). These tests need it." >&2
  exit 1
fi

export MONGOSTUDIO_IT=1
# Anything after `--` is passed to the test binaries; default keeps container load moderate.
extra=("--test-threads=4")
if [ "${1:-}" = "--" ]; then
  shift
  extra=("$@")
fi

status=0
run() { "$@" || status=$?; }
run cargo test -p mongostudio-core --test docker_mongo_auth --test docker_runtime_failures \
  --test docker_ssh_tunnel --locked -- --include-ignored --nocapture "${extra[@]}"
run cargo test -p mongostudio-ui --test docker_e2e_failures --locked -- --include-ignored \
  --nocapture "${extra[@]}"

echo
echo "Leftover test containers (should be none):"
docker ps -a --filter 'name=^ms-it-' --format '{{.Names}}' || true
echo "Safe cleanup if any remain (removes only ms-it-* containers):"
echo "  docker ps -a --filter 'name=^ms-it-' -q | xargs docker rm -f -v"
exit "$status"
