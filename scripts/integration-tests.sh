#!/usr/bin/env bash
# Tier 1 "local server" tests: need ONLY a MongoDB server that is already running. No docker.
#
#   scripts/integration-tests.sh                    # uses mongodb://127.0.0.1:27017
#   CURLYBRACE_IT_MONGODB_URI=mongodb://127.0.0.1:27018 scripts/integration-tests.sh
#
# The tests WRITE to the server, but only to throwaway databases named cbit_<id>, which each test
# drops when it finishes. Point this at a throwaway server, never at production. Extra arguments
# are passed to the test binaries (e.g. a test name filter).
set -euo pipefail

cd "$(dirname "$0")/.."
uri="${CURLYBRACE_IT_MONGODB_URI:-mongodb://127.0.0.1:27017}"

die() { echo "$*" >&2; exit 2; }

# Splits a direct mongodb:// URI into HOST and PORT. A URI this cannot understand is reported as
# such (exit 2), never as an unreachable server.
parse_uri() {
  local u="$1" rest authority hosts hostport
  case "$u" in
    mongodb+srv://*) die "CURLYBRACE_IT_MONGODB_URI is a mongodb+srv:// URI: tier 1 needs a direct mongodb://host:port URI." ;;
    mongodb://*) rest="${u#mongodb://}" ;;
    *) die "CURLYBRACE_IT_MONGODB_URI must start with mongodb:// (got: ${u})" ;;
  esac
  authority="${rest%%[/?]*}"      # drop /db and ?options
  hosts="${authority##*@}"        # drop user:password@
  hostport="${hosts%%,*}"         # first host of a seed list
  [ -n "$hostport" ] || die "No host in CURLYBRACE_IT_MONGODB_URI (${u})"
  if [[ "$hostport" == \[* ]]; then   # [::1] or [::1]:27017
    HOST="${hostport%%]*}"; HOST="${HOST#[}"
    local after="${hostport#*]}"
    PORT="${after#:}"
    [ -n "$after" ] || PORT=27017
  elif [[ "$hostport" == *:* ]]; then
    HOST="${hostport%:*}"; PORT="${hostport##*:}"
  else
    HOST="$hostport"; PORT=27017
  fi
  [ -n "$HOST" ] || die "No host in CURLYBRACE_IT_MONGODB_URI (${u})"
  [[ "$PORT" =~ ^[0-9]+$ ]] || die "Bad port '${PORT}' in CURLYBRACE_IT_MONGODB_URI (${u})"
}

# Plain TCP connect (no mongosh needed), abandoned after 5 seconds.
tcp_open() {
  (exec 3<>"/dev/tcp/$1/$2") 2>/dev/null &
  local pid=$! waited=0
  while kill -0 "$pid" 2>/dev/null; do
    if [ "$waited" -ge 50 ]; then kill "$pid" 2>/dev/null || true; return 1; fi
    sleep 0.1; waited=$((waited + 1))
  done
  wait "$pid"
}

parse_uri "$uri"
if [ "${CURLYBRACE_PARSE_ONLY:-}" = 1 ]; then echo "$HOST $PORT"; exit 0; fi # for the parser check
if ! tcp_open "$HOST" "$PORT"; then
  cat >&2 <<MSG
No MongoDB server is reachable at ${uri} (tried host ${HOST}, port ${PORT}).
Start one, for example:
  mongod --dbpath "\$(mktemp -d)" --bind_ip 127.0.0.1 --port 27017
  docker run -d --rm -p 127.0.0.1:27017:27017 mongo:7
or set CURLYBRACE_IT_MONGODB_URI to another address.
MSG
  exit 1
fi

export CURLYBRACE_IT=1
export CURLYBRACE_IT_MONGODB_URI="$uri"
echo "Tier 1: running against ${uri} (writes only to cbit_* databases)"

failed=()
run() { # run <label> <command...>: keep going so every target runs, then report
  local label="$1"; shift
  "$@" || failed+=("$label")
}
run "core local_mongo_crud" \
  cargo test -p curlybrace-core --test local_mongo_crud --locked -- --include-ignored --nocapture "$@"
run "core local_compare_store" \
  cargo test -p curlybrace-core --test local_compare_store --locked -- --include-ignored --nocapture "$@"
run "core local_compare_session" \
  cargo test -p curlybrace-core --test local_compare_session --locked -- --include-ignored --nocapture "$@"
run "ui local_e2e_flows" \
  cargo test -p curlybrace-ui --test local_e2e_flows --locked -- --include-ignored --nocapture "$@"
run "ui local_compare_flows" \
  cargo test -p curlybrace-ui --test local_compare_flows --locked -- --include-ignored --nocapture "$@"

if [ "${#failed[@]}" -ne 0 ]; then
  echo >&2
  echo "FAILED tier 1 targets: ${failed[*]}" >&2
  exit 1
fi
echo "Tier 1: all targets passed"
