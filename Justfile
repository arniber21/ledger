# Canonical tasks for building, running and demoing Ledger Sentinel. `just` lists them.
# Recipes marked "(pending #N)" exit non-zero until that GitHub issue lands.

set dotenv-load := true
set shell := ["bash", "-euo", "pipefail", "-c"]

validator_dir := ".ledger/validator"

default:
    @just --list

# --- build ---

check:
    cargo check --workspace --all-targets

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

# --- infrastructure ---

# Start TimescaleDB (needs Docker/OrbStack running). Compose file lands with #4.
db-up:
    docker compose up -d --wait timescaledb

db-down:
    docker compose down

# Apply migrations (pending #4).
migrate:
    @echo "migrate: pending #4" >&2; exit 1

# Local Solana validator used for exchange settlement. Foreground; run in its own terminal.
validator:
    mkdir -p {{validator_dir}}
    solana-test-validator --reset --ledger {{validator_dir}}

# --- run ---

# HTTP/SSE API serving verdicts to the UI (pending #9).
api:
    cargo run -p ledger-api

# Start the buying agents; pass faults to inject, e.g. `just agents double-pay no-approval` (pending #7).
agents *faults:
    @echo "agents {{faults}}: pending #7" >&2; exit 1

# Toggle a fault on the running agents (pending #7).
fault name:
    @echo "fault {{name}}: pending #7" >&2; exit 1

# Web UI dev server (pending #14).
ui:
    @echo "ui: pending #14" >&2; exit 1

# Run the SQL time-window query shown in the demo (pending #8).
window-query:
    @echo "window-query: pending #8" >&2; exit 1

# --- demo ---

# Bring up everything for the partner demo in one command (pending #19).
demo:
    @echo "demo: pending #19" >&2; exit 1
