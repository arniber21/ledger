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

Tasks run through [`just`](https://github.com/casey/just); `just` lists them. Recipes whose issue hasn't landed exit with "pending #N".

```
just data         # download company-001 (gitignored)
just check
just db-up        # TimescaleDB on localhost:5433
just migrate      # events hypertable and verdicts table
just validator    # local Solana validator (own terminal)
just demo         # everything, once #19 lands
```

`docker-compose.yml` runs `timescale/timescaledb`. The password in that file (`ledger_local`) is for this local database only. `just test` includes a store insert/read round-trip and needs the database up (`just db-up`).

## Disclosure

- **Sentinel is vendored in vendor/sentinel.** (`sentinel-model`, `sentinel-compile`, `sentinel-core`, plus `sentinel-ingest`, which `sentinel-core` path-depends on for its tests). Crates.io dependencies stay external. All other code in this repository is new work with a dated commit history.
- **Dataset:** QuickBooks-Hackathon-2026 (HuggingFace `CL251215/QuickBooks-Hackathon-2026`), synthetic, fetched by `just data` and not redistributed in this repo. Any modification to it will be listed here. The `hidden/` ground truth is not used.
- **Jev:** thresholds are fixed before looking at results; probabilities are scores, not calibrated confidence. Failures and timeouts become `Inconclusive`. Any Jev failures observed will be listed here.

## Jev

One live System One call: `POST https://api.typesafe.ai/v1/systemone`, requested model `jev-latest`, response model `jev-1.13.0`. Question id `same_obligation` asked whether two bill records are the same obligation (Choice `same` or `different`): vendor "Acme Industrial Supply", invoice `INV-1044`, amount `1250.00` USD, versus vendor "Acme Ind. Supply", invoice `1044`, same amount, different memo wording. Raw response: `fixtures/jev/same-obligation.json`. Request body: `crates/ledger-jev/smoke_request.json`.

Thresholds were fixed in `crates/ledger-jev/src/lib.rs` before that response was read. `P(same) >= 0.80` maps to `Same`. `P(different) >= 0.80` maps to `Different`. A timeout (30s), an error, a score below its threshold, or a contradiction maps to `Unknown`. The model was not asked for explanation text.

## Spec authoring

A policy sentence is drafted with Gemini, checked against the vendored Sentinel catalogue, then judged by Jev. Catalogue validation runs first. Only a catalogue-valid draft that Jev maps to `Same` starts a monitor. `Different` rejects the draft. A timeout, error, score below the thresholds above, or a contradiction maps to `Unknown` and also rejects it. Rejection reasons are codes. The models are not asked for explanation text.

One live Gemini Interactions call drafted "After an agent pays a bill, no second payment may occur on that bill until a credit closes it." `POST https://generativelanguage.googleapis.com/v1beta/interactions`, model `gemini-3.8-flash`, `store` false. Response model `gemini-3.8-flash`. Raw response: `fixtures/gemini/policy-draft.json`. Request: `fixtures/gemini/policy-draft-request.json`.

One live System One call then asked whether that draft matches the sentence (Choice `same` or `different`, question id `draft_matches_policy`). Requested model `jev-latest`, response model `jev-1.13.0`. Raw response: `fixtures/jev/spec-match.json`. Request: `fixtures/jev/spec-match-request.json`. Mapping reuses the `ledger-jev` thresholds; they were not changed for this call.
