# Ledger Sentinel

Ledger Sentinel is a live verification platform for AI agents that handle money. It watches simulated buying agents pay a company's real bills and trade on a Solana-settled exchange, checks every action against plain-English policies as it happens, and turns violations into evidence-backed verdicts with a counterexample timeline you can inspect.

Everything is simulated: no real money, bank, or accounting account is involved. Findings are labeled "needs review", not conclusions about intent.

## Quick start

**Prerequisites**

- Rust (stable) and [`just`](https://github.com/casey/just)
- [Solana CLI](https://solana.com/docs/intro/installation) (provides `solana-test-validator`)
- Docker (for TimescaleDB)
- [bun](https://bun.sh) (for the web UI)

**Configure**

Create `.env` in the repo root. It is gitignored.

```
GEMINI_API_KEY=...      # spec drafting
TYPESAFE_API_KEY=...    # Jev (TypeSafe System One)
SENTRY_DSN=...          # traces, logs, errors
```

**Run**

```sh
just up      # Timescale, validator, API, agents, and the UI
just demo    # reset, then the scripted scenes in order
just scene clean
just reset
```

`just up` downloads company-001 if `data/` does not already have it, starts TimescaleDB, applies migrations, starts the local Solana validator, the API, the buying agents, and the web UI, then waits. Ctrl-C stops the processes that command started. A validator answering RPC on port 8899, an API answering `GET /health` on port 8080, or a UI answering on port 5173 is left running. If one of those ports is taken and does not answer, the command stops. The script is `scripts/demo.sh`. The presenter notes are `docs/demo.md`.

`just reset` stops the agents, truncates the Timescale tables, removes `.ledger/validator`, and clears generated state under `.ledger/`. `just demo` runs that reset, then the scenes in order: `clean`, `double-pay`, `hidden-memo`, `vendor-alias`, `recurring`, `gc-soak`, `spec-live`. Each scene prints one line of the verdicts it expects. `LEDGER_JEV=replay` (the default) reads `fixtures/jev/` and does not call out. `LEDGER_JEV=live` uses System One. A missing replay fixture fails.

The UI is at http://127.0.0.1:5173. Vite proxies `/api` and `/v1` to the API on port 8080. Set `VITE_SENTINEL_SSE` only when the verdict stream is somewhere else.

Individual tasks, if you want one process in its own terminal:

```sh
just data         # download company-001 into data/
just db-up        # TimescaleDB on localhost:5433
just migrate      # events hypertable, verdicts, inbox, and calibration view
just validator    # local Solana validator
just api          # HTTP and SSE on 127.0.0.1:8080
just agents       # buying desk; optional fault names
just ui           # web UI on port 5173
just fault double-pay
just scene double-pay
```

`just check`, `just test`, `just lint`, and `just fmt` are the build tasks. Run `just` to list everything. `just test` includes a database round-trip and needs `just db-up` first.

## How it works

1. **Agents act.** Rig buying agents, with Gemini as the model, pay one company's bills and place orders on the exchange. Faults can be injected live: a double payment, a payment with no approval, a budget breach, a swapped vendor name, or an instruction hidden in a bill memo.
2. **Everything emits a span.** Agents and the exchange are instrumented with OpenTelemetry. The same spans go to Sentry and to our ingest, which normalizes them into events with source and causal links, and stores them in TimescaleDB.
3. **Specs run continuously.** Policies such as "a bill's approval comes before its payment" run as monitors, one per open bill, evaluated incrementally as events arrive. Each ends as `Satisfy`, `Violate` or `Inconclusive`. Missing evidence is never counted as a pass.
4. **Jev handles the ambiguous parts.** When code cannot decide, such as whether two bill or vendor references are the same obligation, whether a payment fits a recurring schedule, or whether a memo is instructing a payment, Jev returns a typed judgment that feeds the monitor. Amount, id, and currency checks decide a binding first and skip Jev when they decide. Same bill id and same period decide a duplicate payment before Jev. Weekly rent is not a duplicate. A second payment of the same bill needs review. Only a `Same` judgment merges monitor keys. An uncertain answer yields `Inconclusive` and leaves the keys apart. Exact-match and fuzzy baselines are stored with each Jev call for the verdict detail.
5. **People review.** Verdicts land in an inbox (board, list or split view) with the evidence, the Solana transaction, and a link to the Sentry trace when `SENTRY_DSN` is set. Confirmations and dismissals are counted by Jev score band on the dashboard. Those counts do not move the fixed thresholds. A violate is needs review.

## Sentinel

`vendor/sentinel` is pre-existing code (Emet Labs). This repo vendors it and does not change it. The new work is the ledger crates and `web/`: the adapter, ingest, exchange, agents, Jev and Gemini calls, API, and UI. Specs are evaluated with the incremental `Evaluator::step` and `seal` path. The vendored workspace is marked `license = "Proprietary"`. Say that in the submission. Devpost allows pre-existing work when it is open source; this tree is not declared open source, so it is not confirmed as allowed under that rule. Details are in `docs/devpost.md`.

## Jev failures

Deterministic checks on amount, id, and currency run first and skip Jev when they decide. Otherwise Jev returns a typed judgment. A timeout (30 seconds), transport error, HTTP error, or unreadable body becomes `Unknown`. A score below the fixed 0.80 threshold, or a contradiction, is also `Unknown`. `Unknown` yields `Inconclusive`, does not merge monitor keys, and is not a pass. Missing evidence is `Inconclusive`. The model generates no explanation text. Do not invent a Jev result.

## Window query

`just window-query` counts stored events per minute over the last hour:

```sql
SELECT time_bucket('1 minute', observed_at) AS bucket,
       source,
       kind,
       count(*) AS events
FROM events
WHERE observed_at >= now() - interval '1 hour'
  AND observed_at < now()
GROUP BY bucket, source, kind
ORDER BY bucket, source, kind;
```

## Tech stack

- **Core** (Rust workspace, eight crates)
  - Rust, Tokio
  - `rust_decimal` for money, `serde` and `serde_yaml` for data
- **Agents** (`ledger-agents`)
  - Rig agents (`rig-core`) for company-001, Gemini `gemini-3.8-flash`
  - Live fault injection: double payment, missing approval, budget breach, swapped vendor, hidden memo instruction
- **Exchange** (`ledger-exchange`)
  - Rust limit-order book for a flour/USD pair
  - Solana: `solana-test-validator`, one SPL token transfer per fill
  - Sentry tracing and logs on submit, match, fill and settlement
- **Telemetry and ingest** (`ledger-ingest`)
  - OpenTelemetry spans from agents and exchange
  - Sentry (Rust SDK with tracing and logs) as one destination, our ingest as the other
  - Normalization into events with source and causal links
- **Verification** (`ledger-core`, `vendor/sentinel`)
  - Sentinel core: compiles specs and evaluates event streams incrementally
  - Spec patterns: precedence, response, absence-after, scoped per bill
  - Verdicts: `Satisfy`, `Violate`, `Inconclusive`
- **Semantic judgments** (`ledger-jev`, `ledger-spec`)
  - Jev (TypeSafe System One): vendor and bill identity, memo predicates, spec-versus-sentence checks, monitor retirement
  - Gemini: drafts specs from plain-English policies
- **Storage** (`ledger-store`)
  - TimescaleDB (Postgres 16): events hypertable, `time_bucket` queries, calibration view over stored review events
  - `sqlx`
- **API** (`ledger-api`)
  - HTTP and Server-Sent Events at `GET /v1/verdicts/stream`
  - Live traces at `GET /api/traces` and `GET /api/traces/stream`
- **Web UI** (`web/`)
  - React, Vite, TypeScript, bun
  - TanStack Router, Query and Table
  - `cmdk` command palette
- **Data**
  - QuickBooks Hackathon 2026 (Intuit), company-001 bills, vendors and payments
- **Tooling**
  - `just`, Docker Compose

## Data

`just data` downloads the dataset from [HuggingFace](https://huggingface.co/datasets/CL251215/QuickBooks-Hackathon-2026) and extracts company-001 into `data/`, which is gitignored. The dataset has no declared license, so it is not committed. The loader does not modify those rows, and it refuses to read a path whose component is `hidden/`. Live faults are injected in the running agents. YAML under `crates/ledger-agents/fixtures/` is synthetic and is not the sponsor dataset.

## Status

`just up` starts the local stack. `just demo` resets that stack and runs the scripted scenes. Findings are labeled "needs review". Neither command finds or records a Sentry incident. Submission notes are in `docs/devpost.md`.
