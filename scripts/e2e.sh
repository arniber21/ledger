#!/usr/bin/env bash
# Run the Playwright harness. Starts or reuses the stack through `just e2e-stack`.
# LEDGER_JEV=replay and LEDGER_TEST=1. Does not print secrets, read hidden/,
# or stop processes it did not start.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

export PATH="${HOME}/.local/share/solana/install/active_release/bin:${HOME}/.local/bin:${PATH}"
export LEDGER_TEST=1
export LEDGER_JEV="${LEDGER_JEV:-replay}"
export DATABASE_URL="${DATABASE_URL:-postgres://ledger:ledger_local@127.0.0.1:5433/ledger_e2e}"
# shellcheck disable=SC1091
source "${root}/scripts/e2e-slot.sh"

port_free() {
  python3 -c 'import socket, sys
s = socket.socket()
s.settimeout(0.2)
code = s.connect_ex(("127.0.0.1", int(sys.argv[1])))
raise SystemExit(0 if code != 0 else 1)' "$1"
}

pick_port() {
  local prefer="$1"
  shift
  if port_free "$prefer"; then
    printf '%s\n' "$prefer"
    return
  fi
  python3 -c 'import socket, sys
skip = {int(item) for item in sys.argv[1:]}
for _ in range(200):
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
    sock.close()
    if port not in skip:
        print(port)
        raise SystemExit(0)
raise SystemExit("no free port")' "$@" "$prefer"
}

http_ok() {
  curl -fsS --max-time 2 "$1" >/dev/null 2>&1
}

reused=0
if [[ -f .ledger/e2e/env ]]; then
  if [[ -n "${LEDGER_E2E_SLOT:-}" ]]; then
    saved_origin="$(grep '^LEDGER_API_ORIGIN=' .ledger/e2e/env | cut -d= -f2-)"
    saved_ui="$(grep '^LEDGER_UI_URL=' .ledger/e2e/env | cut -d= -f2-)"
    if [[ "$saved_origin" == "$LEDGER_API_ORIGIN" && "$saved_ui" == "$LEDGER_UI_URL" ]] \
      && http_ok "${LEDGER_UI_URL}/" \
      && curl -fsS --max-time 2 "${LEDGER_API_ORIGIN}/api/test" | grep -q '"enabled":true'; then
      reused=1
    fi
  elif grep -q '^LEDGER_E2E_SLOT=' .ledger/e2e/env; then
    # A previous slot run recorded its own ports. Leave that stack alone.
    :
  else
    # shellcheck disable=SC1091
    source .ledger/e2e/env
    if [[ -n "${LEDGER_UI_URL:-}" && -n "${LEDGER_API_ORIGIN:-}" ]] \
      && http_ok "${LEDGER_UI_URL}/" \
      && curl -fsS --max-time 2 "${LEDGER_API_ORIGIN}/api/test" | grep -q '"enabled":true'; then
      reused=1
    fi
  fi
fi

if [[ "$reused" != 1 && -z "${LEDGER_E2E_SLOT:-}" ]]; then
  # 5174 and 8091 have failed in this environment. Prefer the documented ports,
  # then any other free port.
  skip=(5174 8091 5433 8899)
  LEDGER_API_PORT="$(pick_port 8080 "${skip[@]}")"
  LEDGER_UI_PORT="$(pick_port 5173 "${skip[@]}" "$LEDGER_API_PORT")"
  export LEDGER_API_PORT LEDGER_UI_PORT
  export LEDGER_API_ADDR="127.0.0.1:${LEDGER_API_PORT}"
  export LEDGER_API_ORIGIN="http://127.0.0.1:${LEDGER_API_PORT}"
  export LEDGER_UI_URL="http://127.0.0.1:${LEDGER_UI_PORT}"
fi

export LEDGER_API_PORT LEDGER_UI_PORT LEDGER_API_ADDR LEDGER_API_ORIGIN LEDGER_UI_URL

mkdir -p .ledger/e2e
{
  echo "LEDGER_API_PORT=${LEDGER_API_PORT}"
  echo "LEDGER_UI_PORT=${LEDGER_UI_PORT}"
  echo "LEDGER_API_ADDR=${LEDGER_API_ADDR}"
  echo "LEDGER_API_ORIGIN=${LEDGER_API_ORIGIN}"
  echo "LEDGER_UI_URL=${LEDGER_UI_URL}"
  echo "DATABASE_URL=${DATABASE_URL}"
  if [[ -n "${LEDGER_E2E_SLOT:-}" ]]; then
    echo "LEDGER_E2E_SLOT=${LEDGER_E2E_SLOT}"
    echo "LEDGER_COMPOSE_PROJECT=${LEDGER_COMPOSE_PROJECT}"
    echo "LEDGER_DB_PORT=${LEDGER_DB_PORT}"
  fi
} >.ledger/e2e/env

stop_recorded() {
  local file pid
  for file in .ledger/e2e/*.pid; do
    [[ -f "$file" ]] || continue
    pid="$(cat "$file")"
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill -TERM -"$pid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true
    fi
  done
  sleep 0.3
  for file in .ledger/e2e/*.pid; do
    [[ -f "$file" ]] || continue
    pid="$(cat "$file")"
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill -KILL -"$pid" 2>/dev/null || kill -KILL "$pid" 2>/dev/null || true
    fi
  done
}

cd web
bun install
bunx playwright install chromium
set +e
bunx playwright test --config e2e/playwright.config.ts "$@"
code=$?
set -e
cd "$root"
stop_recorded
exit "$code"
