#!/usr/bin/env bash
# Start TimescaleDB, the Solana validator, the API, the buying agents, and the UI
# for the Playwright harness. Ctrl-C stops only the processes this command started.
# A port that is already open is left alone. LEDGER_JEV=replay. LEDGER_TEST=1.
# This command does not record a Sentry incident and does not read hidden/.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

export PATH="${HOME}/.local/share/solana/install/active_release/bin:${HOME}/.local/bin:${PATH}"
export LEDGER_TEST=1
export LEDGER_JEV="${LEDGER_JEV:-replay}"
export DATABASE_URL="${DATABASE_URL:-postgres://ledger:ledger_local@127.0.0.1:5433/ledger_e2e}"
# shellcheck disable=SC1091
source "${root}/scripts/e2e-slot.sh"

if [[ -z "${LEDGER_API_PORT:-}" || -z "${LEDGER_UI_PORT:-}" ]]; then
  printf 'e2e: LEDGER_API_PORT and LEDGER_UI_PORT are unset. Run just e2e.\n' >&2
  exit 1
fi

export LEDGER_API_ADDR="${LEDGER_API_ADDR:-127.0.0.1:${LEDGER_API_PORT}}"
export LEDGER_API_ORIGIN="${LEDGER_API_ORIGIN:-http://127.0.0.1:${LEDGER_API_PORT}}"
export LEDGER_UI_URL="${LEDGER_UI_URL:-http://127.0.0.1:${LEDGER_UI_PORT}}"

log_dir=".ledger/e2e"
mkdir -p "$log_dir"
started_pids=()

say() {
  printf 'e2e: %s\n' "$1"
}

die() {
  printf 'e2e: %s\n' "$1" >&2
  exit 1
}

cleanup() {
  local pid
  for pid in ${started_pids[@]+"${started_pids[@]}"}; do
    if kill -0 "$pid" 2>/dev/null; then
      kill -TERM -"$pid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true
    fi
  done
  sleep 0.3
  for pid in ${started_pids[@]+"${started_pids[@]}"}; do
    if kill -0 "$pid" 2>/dev/null; then
      kill -KILL -"$pid" 2>/dev/null || kill -KILL "$pid" 2>/dev/null || true
    fi
  done
  started_pids=()
}

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing $1"
}

need_key() {
  local name="$1"
  local value="${!name-}"
  if [[ -z "${value//[[:space:]]/}" ]]; then
    die "${name} is unset. Add it to .env in the repo root. The value is not printed."
  fi
}

port_open() {
  python3 -c 'import socket, sys
s = socket.socket()
s.settimeout(1)
code = s.connect_ex(("127.0.0.1", int(sys.argv[1])))
raise SystemExit(0 if code == 0 else 1)' "$1"
}

validator_ready() {
  curl -fsS --max-time 2 \
    -H 'content-type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getVersion"}' \
    http://127.0.0.1:8899 >/dev/null 2>&1
}

http_ok() {
  curl -fsS --max-time 2 "$1" >/dev/null 2>&1
}

agents_live() {
  python3 -c 'import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(1)
try:
    s.connect(".ledger/agents.sock")
except OSError:
    raise SystemExit(1)
s.close()'
}

last_pid=""
start_bg() {
  local name="$1"
  shift
  say "starting ${name}"
  python3 -c 'import os, sys
os.setsid()
os.execvp(sys.argv[1], sys.argv[1:])' "$@" >"${log_dir}/${name}.log" 2>&1 &
  last_pid=$!
  started_pids+=("$last_pid")
  printf '%s\n' "$last_pid" >"${log_dir}/${name}.pid"
}

wait_for() {
  local label="$1"
  local seconds="$2"
  local pid="$3"
  local check="$4"
  shift 4
  local i
  for ((i = 1; i <= seconds; i++)); do
    if [[ -n "$pid" ]] && ! kill -0 "$pid" 2>/dev/null; then
      die "${label} exited before it was ready. Log: ${log_dir}/${label}.log"
    fi
    if "$check" "$@"; then
      say "${label} ready"
      return 0
    fi
    sleep 1
  done
  die "${label} was not ready after ${seconds}s. Log: ${log_dir}/${label}.log"
}

if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi
# A slot wins over .env so this stack does not attach to the default database.
# shellcheck disable=SC1091
source "${root}/scripts/e2e-slot.sh"

need_cmd docker
need_cmd solana-test-validator
need_cmd bun
need_cmd cargo
need_cmd curl
need_cmd python3
need_cmd just
docker info >/dev/null 2>&1 || die "Docker is not running. Start OrbStack or Docker, then run just e2e again."

need_key GEMINI_API_KEY
need_key TYPESAFE_API_KEY
if [[ -z "${SENTRY_DSN-}" || -z "${SENTRY_DSN//[[:space:]]/}" ]]; then
  say "SENTRY_DSN is unset. The stack still starts. Traces are not sent to Sentry. This command does not record a Sentry incident."
fi

if [[ ! -d data/hackathon-100/company-001/rwm-v4 ]]; then
  say "company-001 is not in data/. Downloading it."
  just data
fi

if validator_ready; then
  say "validator already ready at http://127.0.0.1:8899"
else
  validator_pid=""
  if port_open 8899; then
    say "port 8899 is open. Waiting for Solana RPC."
  else
    mkdir -p .ledger/validator
    start_bg validator \
      solana-test-validator \
      --reset \
      --ledger .ledger/validator \
      --rpc-port 8899 \
      --gossip-port 18000 \
      --dynamic-port-range 18010-18100
    validator_pid="$last_pid"
  fi
  wait_for validator 90 "$validator_pid" validator_ready
fi

say "compose project ${LEDGER_COMPOSE_PROJECT:-ledger}, database port ${LEDGER_DB_PORT:-5433}"
say "starting TimescaleDB"
just db-up
exists="$(docker compose exec -T timescaledb psql -U ledger -d postgres -tAc "SELECT 1 FROM pg_database WHERE datname = 'ledger_e2e'")"
if [[ "${exists//[[:space:]]/}" != "1" ]]; then
  say "creating database ledger_e2e"
  docker compose exec -T timescaledb psql -U ledger -d postgres -v ON_ERROR_STOP=1 -c "CREATE DATABASE ledger_e2e"
fi

say "applying migrations to ledger_e2e"
just migrate

say "building the API, scene runner, and agents"
cargo build -p ledger-api -p ledger-scene -p ledger-agents

if http_ok "${LEDGER_API_ORIGIN}/health" && curl -fsS --max-time 2 "${LEDGER_API_ORIGIN}/api/test" | grep -q '"enabled":true'; then
  say "API already ready at ${LEDGER_API_ORIGIN}"
else
  if port_open "$LEDGER_API_PORT"; then
    die "port ${LEDGER_API_PORT} is open, but the test API did not answer. Leave that process running and run just e2e again."
  fi
  start_bg api just api
  wait_for api 600 "$last_pid" http_ok "${LEDGER_API_ORIGIN}/health"
fi

if agents_live; then
  say "agents already listening on .ledger/agents.sock"
else
  if [[ -S .ledger/agents.sock ]]; then
    rm -f .ledger/agents.sock
  fi
  start_bg agents just agents
  wait_for agents 600 "$last_pid" agents_live
fi

if http_ok "${LEDGER_UI_URL}/"; then
  say "UI already ready at ${LEDGER_UI_URL}"
else
  if port_open "$LEDGER_UI_PORT"; then
    die "port ${LEDGER_UI_PORT} is open, and the dev server did not respond. Leave that process running and run just e2e again."
  fi
  say "installing UI dependencies"
  (cd web && bun install)
  start_bg ui bash -c "cd web && exec bun run dev -- --host 127.0.0.1 --port ${LEDGER_UI_PORT} --strictPort"
  wait_for ui 180 "$last_pid" http_ok "${LEDGER_UI_URL}/"
fi

say "UI ${LEDGER_UI_URL}"
say "API ${LEDGER_API_ORIGIN}"
say "logs are in .ledger/e2e/"

while true; do
  for pid in ${started_pids[@]+"${started_pids[@]}"}; do
    if ! kill -0 "$pid" 2>/dev/null; then
      die "a started process exited. Logs: ${log_dir}/"
    fi
  done
  sleep 2
done
