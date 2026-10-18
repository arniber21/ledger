# Canonical tasks for building, running and demoing Ledger Sentinel. `just` lists them.
# Recipes marked "(pending #N)" exit non-zero until that GitHub issue lands.

set dotenv-load := true
set shell := ["bash", "-euo", "pipefail", "-c"]
set export := true

PATH := env_var("HOME") + "/.local/share/solana/install/active_release/bin:" + env_var("HOME") + "/.local/bin:" + env_var("PATH")

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

# Apply the events hypertable and verdicts table migrations.
migrate: db-up
    cargo run -p ledger-store --bin ledger-migrate

# Local Solana validator used for exchange settlement. Foreground; run in its own terminal.
# Gossip and the dynamic port range sit above 18000 so a process already bound to 8000
# does not stop the validator from starting. RPC stays on 8899.
validator:
    mkdir -p {{validator_dir}}
    solana-test-validator --reset --ledger {{validator_dir}} --gossip-port 18000 --dynamic-port-range 18010-18100

# --- run ---

# HTTP/SSE API serving verdicts to the UI (pending #9).
api:
    cargo run -p ledger-api

# Start the buying agents. Optional fault names turn those faults on.
# Example: just agents double-pay no-approval
agents *faults:
    cargo run -p ledger-agents --bin ledger-agents -- {{faults}}

# Toggle a fault on the running agents.
fault name:
    cargo run -p ledger-agents --bin ledger-agents -- --toggle {{name}}

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
