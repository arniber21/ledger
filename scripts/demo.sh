#!/usr/bin/env bash
# Start TimescaleDB, the local Solana validator, the API, the buying agents, and the web UI.
# Ctrl-C stops only the processes this command started. A port that is already open is left alone.
# This command does not record a Sentry incident.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

export PATH="${HOME}/.local/share/solana/install/active_release/bin:${HOME}/.local/bin:${PATH}"

if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi

log_dir=".ledger/demo"
mkdir -p "$log_dir"
started_pids=()

say() {
  printf 'demo: %s\n' "$1"
}

die() {
  printf 'demo: %s\n' "$1" >&2
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

# Name the process that holds a TCP port: PID, command, cwd, and how to stop it.
describe_port_holder() {
  local port="$1" pid cwd cmd
  pid="$(lsof -nP -iTCP:"$port" -sTCP:LISTEN -t 2>/dev/null | head -n1 || true)"
  if [[ -z "$pid" ]]; then
    printf 'held by a process I could not identify (try: lsof -nP -iTCP:%s -sTCP:LISTEN)' "$port"
    return 0
  fi
  cwd="$(lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p' | head -n1 || true)"
  cmd="$(ps -o command= -p "$pid" 2>/dev/null | cut -c1-120 || true)"
  printf 'held by PID %s (%s) with cwd %s. Stop it with: kill %s (or run just reset to stop stale ledger processes)' \
    "$pid" "$cmd" "${cwd:-unknown}" "$pid"
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
  # New session so Ctrl-C can stop cargo and the binary it spawned.
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

need_cmd docker
need_cmd solana-test-validator
need_cmd bun
need_cmd cargo
need_cmd curl
need_cmd python3
need_cmd just
docker info >/dev/null 2>&1 || die "Docker is not running. Start OrbStack or Docker, then run just demo again."

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

say "starting TimescaleDB and applying migrations"
just migrate

if http_ok http://127.0.0.1:8080/health; then
  say "API already ready at http://127.0.0.1:8080"
else
  if port_open 8080; then
    die "port 8080 is open, but GET /health did not return ok; it is $(describe_port_holder 8080). Then run just up again."
  fi
  say "the first API build can take several minutes"
  start_bg api cargo run -p ledger-api
  wait_for api 600 "$last_pid" http_ok http://127.0.0.1:8080/health
fi

if agents_live; then
  say "agents already listening on .ledger/agents.sock"
else
  say "the first agent build can take several minutes"
  start_bg agents cargo run -p ledger-agents --bin ledger-agents
  wait_for agents 600 "$last_pid" agents_live
fi

if http_ok http://127.0.0.1:5173/; then
  say "UI already ready at http://127.0.0.1:5173"
else
  if port_open 5173; then
    die "port 5173 is open, and the dev server did not respond; it is $(describe_port_holder 5173). Then run just up again."
  fi
  say "installing UI dependencies"
  (cd web && bun install --frozen-lockfile)
  start_bg ui bash -c 'cd web && exec bun run dev -- --host 127.0.0.1 --port 5173 --strictPort'
  wait_for ui 180 "$last_pid" http_ok http://127.0.0.1:5173/
fi

say "UI http://127.0.0.1:5173"
say "inbox http://127.0.0.1:5173/inbox"
say "verdicts http://127.0.0.1:8080/v1/verdicts/stream"
say "toggle a fault from another terminal: just fault double-pay"
say "logs are in .ledger/demo/"
say "Ctrl-C stops the processes this command started"

while true; do
  for pid in ${started_pids[@]+"${started_pids[@]}"}; do
    if ! kill -0 "$pid" 2>/dev/null; then
      die "a started process exited. Logs: ${log_dir}/"
    fi
  done
  sleep 2
done
