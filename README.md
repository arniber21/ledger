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
just data         # download the sample company (company-001) into data/
just db-up        # TimescaleDB on localhost:5433
just migrate      # events hypertable and verdicts table
just validator    # local Solana validator (own terminal)
just demo         # agents, API and UI together
just ui           # web UI alone, on port 5173
```

The dev server proxies `GET /v1/verdicts/stream`. Metrics, the inbox, and the Verdicts timeline stay empty until that stream sends verdicts. Set `VITE_SENTINEL_SSE` to use a different stream URL.

Other tasks: `just check`, `just test`, `just lint`, `just fmt`. Run `just` to list everything. `just test` includes a database round-trip and needs `just db-up` first.

## How it works

1. **Agents act.** Rig buying agents, with Gemini as the model, pay one company's bills and place orders on the exchange. Faults can be injected live: a double payment, a payment with no approval, a budget breach, a swapped vendor name, or an instruction hidden in a bill memo.
2. **Everything emits a span.** Agents and the exchange are instrumented with OpenTelemetry. The same spans go to Sentry and to our ingest, which normalizes them into events with source and causal links, and stores them in TimescaleDB.
3. **Specs run continuously.** Policies such as "a bill's approval comes before its payment" run as monitors, one per open bill, evaluated incrementally as events arrive. Each ends as `Satisfy`, `Violate` or `Inconclusive`. Missing evidence is never counted as a pass.
4. **Jev handles the ambiguous parts.** When code cannot decide, such as whether two bill or vendor references are the same obligation, whether a payment fits a recurring schedule, or whether a memo is instructing a payment, Jev returns a typed judgment that feeds the monitor. Amount, id, and currency checks decide a binding first and skip Jev when they decide. Same bill id and same period decide a duplicate payment before Jev. Weekly rent is not a duplicate. A second payment of the same bill needs review. Only a `Same` judgment merges monitor keys. An uncertain answer yields `Inconclusive` and leaves the keys apart. Exact-match and fuzzy baselines are stored with each Jev call for the verdict detail.
5. **People review.** Verdicts land in an inbox (board, list or split view) with the evidence, the Solana transaction, and a link to the Sentry trace. Confirmations and dismissals are counted by Jev score band on the dashboard. Those counts do not move the fixed thresholds. A violate is needs review.

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
  - Jev (TypeSafe System One): vendor and bill identity, memo predicates, spec-versus-sentence checks, monitor retirement, and Quick actions that route a phrase onto Inbox filters, a screen, a verdict already on the stream, or a fault confirmation
  - Gemini: drafts specs from plain-English policies
- **Storage** (`ledger-store`)
  - TimescaleDB (Postgres 16): events hypertable, `time_bucket` queries, calibration view over stored review events
  - `sqlx`
- **API** (`ledger-api`)
  - HTTP and Server-Sent Events at `GET /v1/verdicts/stream`
  - Live traces at `GET /api/traces` and `GET /api/traces/stream`
  - Quick actions at `POST /api/quick-actions`
- **Web UI** (`web/`)
  - React, Vite, TypeScript, bun
  - TanStack Router, Query and Table
  - `cmdk` command palette
- **Data**
  - QuickBooks Hackathon 2026 (Intuit), company-001 bills, vendors and payments
- **Tooling**
  - `just`, Docker Compose

## Data

`just data` downloads the dataset from [HuggingFace](https://huggingface.co/datasets/CL251215/QuickBooks-Hackathon-2026) and extracts company-001 into `data/`, which is gitignored. The simulation's ground-truth files are never read.
