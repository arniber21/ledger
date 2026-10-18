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
```

Other tasks: `just check`, `just test`, `just lint`, `just fmt`. Run `just` to list everything. `just test` includes a database round-trip and needs `just db-up` first.

## How it works

1. **Agents act.** Scripted buying agents for one company pay its bills and place orders on the exchange. Faults can be injected live: a double payment, a payment with no approval, a budget breach, a swapped vendor name, or an instruction hidden in a bill memo.
2. **Everything emits a span.** Agents and the exchange are instrumented with OpenTelemetry. The same spans go to Sentry and to our ingest, which normalizes them into events with source and causal links, and stores them in TimescaleDB.
3. **Specs run continuously.** Policies such as "a bill's approval comes before its payment" run as monitors, one per open bill, evaluated incrementally as events arrive. Each ends as `Satisfy`, `Violate` or `Inconclusive`. Missing evidence is never counted as a pass.
4. **Jev handles the ambiguous parts.** When code cannot decide, such as whether two vendor names refer to the same payee or whether a memo is instructing a payment, Jev returns a typed judgment that feeds the monitor. Deterministic checks run first, and an uncertain answer yields `Inconclusive`.
5. **People review.** Verdicts land in an inbox (board, list or split view) with the evidence, the Solana transaction, and a link to the Sentry trace.

## Tech stack

| Layer | Technology | What it does here |
| --- | --- | --- |
| Language | Rust, Tokio | The exchange, agents, ingest, verification and API are one Cargo workspace of eight crates. |
| Verification engine | Sentinel core (`vendor/sentinel`) | Compiles specs (precedence, response, absence-after, scoped per bill) and evaluates event streams incrementally. Verdicts carry reason codes and the matched evidence. |
| Semantic judgments | **Jev**, TypeSafe's System One model | Typed yes/no and choice judgments on short structured inputs: vendor and bill identity resolution, memo predicates, checking a drafted spec against its sentence, and deciding when a monitor can be retired. Every input and output is stored and shown in the UI. |
| Spec drafting | **Gemini** | Turns a plain-English policy into a draft pattern. Jev checks the draft against the sentence, and only valid patterns are accepted. |
| Exchange | Rust limit-order book, `rust_decimal` | Order submission, matching and fills for a flour/USD pair. |
| Settlement | **Solana** (`solana-test-validator`), SPL token transfers | Every fill settles as a token transfer on a local validator, and the transaction signature is attached to the trace. |
| Observability | **Sentry** (Rust SDK, tracing and logs), OpenTelemetry | Traces, logs and error capture across the exchange, agents, ingest and Jev calls. One trace covers decision, order, fill and settlement. |
| Storage | **TimescaleDB** (Postgres 16) via `sqlx` | Events live in a hypertable. The dashboard charts come from `time_bucket` queries. Verdicts and review status sit alongside. |
| Data | **QuickBooks Hackathon 2026** (Intuit), company-001 | The real bills, vendors and payments the agents work through. Synthetic data, fetched by `just data`. |
| API | HTTP and Server-Sent Events | Streams spans, spec-authoring steps and verdicts to the UI as they happen. |
| Web UI | React, Vite, TypeScript, TanStack Router, Query and Table, `cmdk`, bun | Dashboard, inbox with three view modes, trace and verdict detail, live spec authoring, and a ⌘K command palette. |
| Tooling | `just`, Docker Compose | Task runner and the local database. |

## Data

`just data` downloads the dataset from [HuggingFace](https://huggingface.co/datasets/CL251215/QuickBooks-Hackathon-2026) and extracts company-001 into `data/`, which is gitignored. The simulation's ground-truth files are never read.

## Status

Under active development for LA Hacks 2026. Work is tracked in the repository's issues. The one-command `just demo` is the last piece to land; until then run the pieces individually (`just api`, `just agents`, `just ui`).
