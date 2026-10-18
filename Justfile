# Canonical tasks for building, running and demoing Ledger Sentinel. `just` lists them.

set dotenv-load := true
set shell := ["bash", "-euo", "pipefail", "-c"]
set export := true

PATH := env_var("HOME") + "/.local/share/solana/install/active_release/bin:" + env_var("HOME") + "/.local/bin:" + env_var("PATH")

validator_dir := ".ledger/validator"

default:
    @just --list

# --- build ---

check: web-deps
    cargo check --workspace --all-targets
    cd web && bun run typecheck

# Install web/ dependencies from the lockfile when node_modules is missing (fresh clone).
web-deps:
    [ -d web/node_modules ] || (cd web && bun install --frozen-lockfile)

build:
    cargo build --workspace --all-targets

test:
    cargo test --workspace

lint:
    cargo clippy --workspace --all-targets -- -D warnings

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

# --- data ---

# Download the sponsor dataset (HuggingFace CL251215/QuickBooks-Hackathon-2026) and extract
# company-001 into data/hackathon-100/company-001. Gitignored: the dataset has no declared license.
# Never extracts `hidden/` (simulation ground truth).
data:
    mkdir -p data
    [ -f data/hackathon-100-participant.tar.gz ] || curl -sL --fail -o data/hackathon-100-participant.tar.gz https://huggingface.co/datasets/CL251215/QuickBooks-Hackathon-2026/resolve/main/hackathon-100-participant.tar.gz
    tar -xzf data/hackathon-100-participant.tar.gz -C data --exclude='*hidden*' 'hackathon-100/company-001'
    @echo "data/hackathon-100/company-001"

# --- infrastructure ---

# Start TimescaleDB (needs Docker/OrbStack running).
db-up:
    docker compose up -d --wait timescaledb

db-down:
    docker compose down

# Apply the events hypertable, verdicts, inbox, and calibration view.
migrate: db-up
    cargo run -p ledger-store --bin ledger-migrate

# Local Solana validator used for exchange settlement. Foreground; run in its own terminal.
# Gossip and the dynamic port range sit above 18000 so a process already bound to 8000
# does not stop the validator from starting. RPC stays on 8899.
validator:
    mkdir -p {{validator_dir}}
    solana-test-validator --reset --ledger {{validator_dir}} --gossip-port 18000 --dynamic-port-range 18010-18100

# --- run ---

# HTTP/SSE API.
# Verdicts stream at GET /v1/verdicts/stream.
# Live traces are GET /api/traces and GET /api/traces/stream.
api:
    cargo run -p ledger-api

# Start the buying agents. Optional fault names turn those faults on.
# Example: just agents double-pay no-approval
agents *faults:
    cargo run -p ledger-agents --bin ledger-agents -- {{faults}}

# Toggle a fault on the running agents.
fault name:
    cargo run -p ledger-agents --bin ledger-agents -- --toggle {{name}}

# Web UI dev server. Vite on port 5173. The dashboard stays empty until VITE_SENTINEL_SSE is set.
ui: web-deps
    cd web && bun run dev

# Count stored events per minute over the last hour (Timescale time_bucket).
window-query: migrate
    cargo run -p ledger-ingest --bin ledger-window

# --- demo ---

# Stop agents, truncate Timescale, recreate the validator ledger, clear .ledger state.
reset:
    bash scripts/reset.sh

# Run one scene in replay mode. LEDGER_JEV=live calls System One instead.
scene name:
    LEDGER_JEV="${LEDGER_JEV:-replay}" cargo run -p ledger-scene -- {{name}}

# Reset, then run every scene in order. Each scene prints the verdicts it expects.
demo:
    just reset
    LEDGER_JEV=replay cargo run -p ledger-scene -- --all

# Start TimescaleDB, the Solana validator, the API, the buying agents, and the UI.
# Ctrl-C stops the processes this command started. See docs/demo.md.
up:
    bash scripts/demo.sh

# Playwright against the local stack. Headless. LEDGER_JEV=replay and LEDGER_TEST=1.
e2e: web-deps
    bash scripts/e2e.sh

# The same harness in the Playwright UI.
e2e-ui: web-deps
    bash scripts/e2e.sh --ui

# Database, validator, API, agents, and UI for the Playwright harness.
e2e-stack:
    bash scripts/e2e-stack.sh
