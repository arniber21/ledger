#!/usr/bin/env bash
# Stop agents this repo started, truncate Timescale tables, remove the validator
# ledger, and clear generated state under .ledger/. Does not read hidden/ and
# does not print secrets.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

export PATH="${HOME}/.local/share/solana/install/active_release/bin:${HOME}/.local/bin:${PATH}"

say() {
  printf 'reset: %s\n' "$1"
}

stop_pid_file() {
  local file="$1"
  if [[ -f "$file" ]]; then
    local pid
    pid="$(cat "$file")"
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill -TERM "$pid" 2>/dev/null || true
      sleep 0.2
      kill -KILL "$pid" 2>/dev/null || true
    fi
  fi
}

stop_listeners() {
  local target="$1"
  if [[ ! -e "$target" ]]; then
    return 0
  fi
  local pids
  pids="$(lsof -t "$target" 2>/dev/null || true)"
  if [[ -n "$pids" ]]; then
    # shellcheck disable=SC2086
    kill -TERM $pids 2>/dev/null || true
    sleep 0.2
    # shellcheck disable=SC2086
    kill -KILL $pids 2>/dev/null || true
  fi
}

# Kill ledger-api / ledger-agents whose binary lives under this repo or its worktrees.
# Matches on the executable path, so other checkouts are left alone.
stop_stale_binaries() {
  local name pid exe
  for name in ledger-api ledger-agents; do
    for pid in $(pgrep -x "$name" 2>/dev/null || true); do
      exe="$(ps -o command= -p "$pid" 2>/dev/null | awk '{print $1}')"
      case "$exe" in
        "$root"/target/*|"$root"/.claude/worktrees/*|"$root"/worktrees/*)
          say "stopping stale $name (PID $pid, $exe)"
          kill -TERM "$pid" 2>/dev/null || true
          sleep 0.2
          kill -KILL "$pid" 2>/dev/null || true
          ;;
      esac
    done
  done
}

say "stopping agents"
stop_stale_binaries
stop_pid_file .ledger/demo/agents.pid
if [[ -S .ledger/agents.sock ]]; then
  stop_listeners .ledger/agents.sock
fi

say "stopping a validator whose ledger is .ledger/validator"
stop_pid_file .ledger/demo/validator.pid
if [[ -d .ledger/validator ]]; then
  stop_listeners .ledger/validator
fi

say "clearing generated state"
rm -rf .ledger/demo .ledger/scenes .ledger/validator
rm -f .ledger/agents.sock
mkdir -p .ledger/validator

say "migrating and truncating Timescale tables"
just migrate
docker compose exec -T timescaledb \
  psql -U ledger -d ledger -v ON_ERROR_STOP=1 \
  -c "TRUNCATE TABLE verdict_rank, verdict_triage, verdicts, events;"

say "stack is empty"
