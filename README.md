# Ledger Sentinel

A mini-Sentinel for financial agents: a live runtime-verification platform that watches simulated AI buying agents pay bills and trade on a Solana-settled exchange, judges ambiguous evidence with Jev, and surfaces evidence-backed verdicts (`Satisfy`, `Violate`, `Inconclusive`) with counterexample timelines.

Everything is simulated. No real money, no real bank or QuickBooks account. Findings say "needs review", never "fraud".

See [SPEC.md](SPEC.md) for the full design and decision log; work is tracked in GitHub issues.

## What it does

1. Buying agents act for a company from the QuickBooks hackathon dataset (company-001) and pay its real bills.
2. Agents and the exchange emit OTel spans. One exporter fans out to Sentry and to our ingest.
3. Ingest normalises spans into Sentinel events, stores them in a TimescaleDB hypertable, and feeds the Sentinel core incrementally.
4. Faults are injected live (double-pay, missing approval, budget breach, swapped vendor, hidden memo instruction) and caught as verdicts.
5. Jev resolves ambiguous bindings, judges free-text predicates, checks Gemini-drafted specs, and decides when a monitor is safely retired.
6. A web UI shows metrics, live trace ingest, live spec authoring and live verdicts.

## Layout

```
crates/
  ledger-core      domain types + adapter to Sentinel events
  ledger-exchange  limit-order book + SPL settlement on a local validator
  ledger-agents    simulated buyers + fault injection
  ledger-ingest    OTel span ingest -> canonical events
  ledger-jev       Jev (TypeSafe System One) client
  ledger-spec      Gemini spec drafting, validated against Sentinel Patterns
  ledger-store     TimescaleDB storage
  ledger-api       HTTP/SSE API for the UI
```

## Setup

Requires Rust, the Solana CLI (`solana-test-validator`), and Docker (TimescaleDB).

Create `.env` (gitignored):

```
GEMINI_API_KEY=
TYPESAFE_API_KEY=   # Jev
SENTRY_DSN=
```

```
cargo check
```

## Disclosure

- **Sentinel is pre-existing code** (`sentinel-model`, `sentinel-compile`, `sentinel-core`), consumed as a path dependency from `../../Desktop/emet/sentinel` and not copied into this repo. All code in this repository is new work with a dated commit history.
- **Dataset:** QuickBooks-Hackathon-2026, synthetic. Any modification to it will be listed here. The `hidden/` ground truth is not used.
- **Jev:** thresholds are fixed before looking at results; probabilities are scores, not calibrated confidence. Failures and timeouts become `Inconclusive`. Any Jev failures observed will be listed here.
