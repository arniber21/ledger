# Ledger Sentinel: Spec

A mini-Sentinel for financial agents: a live runtime-verification platform that watches AI buying agents trading and paying bills on a simulated Solana-settled exchange, judges ambiguous evidence with Jev, and surfaces evidence-backed verdicts. (The ASI:One triage agent was cut on 2026-10-18.)

Supersedes the earlier ChatGPT brief. Event: LA Hacks, Oct 17-18 2026, deadline Sun Oct 18 12:00 PM PT (verify on Devpost).

## 1. Decisions

- **Language:** Rust (rich types; Sentinel's core is Rust). No Python.
- **Not agent-powered at its core.** The platform is deterministic live monitoring. The triage agent is a read-heavy layer on top, added after the platform works.
- **Reuse Sentinel by path dependency, not translation.** `sentinel-model`, `sentinel-compile` and `sentinel-core` (~30k lines) build standalone on host cargo (checked, 3.4s, `cargo check -p sentinel-core`). Source: `~/Desktop/emet/sentinel`. Core is a pure evaluator (ADR-0012); use the incremental `Evaluator::step` + `seal` path, not batch `evaluate()`.
- **Disclosure:** Sentinel is pre-existing code. State it in the README and video; keep all new work in this repo with a dated commit history.
- **Solana is last.** Local `solana-test-validator` as a settlement sink (SPL transfer per fill). Install not yet verified. If it slows us, ship a signed-ledger stub and add Solana later.
- **Everything is simulated.** No real money, no real exchange, no unauthorized financial action.

## 2. Architecture

```
QuickBooks dataset ──► company context (vendors, bills, budgets, terms)
                              │
Buying agents (per company) ──┤  decide & pay bills via
                              ▼
Mini exchange (Rust, Solana-settled) ── orders, fills, transfers ──► tx sigs
                              │ every action is an event
                              ▼
Ingest ─► canonical Sentinel Events ─► sentinel-core (live, incremental) ─► Verdicts
              │                              ▲
              ▼                              │ Jev: binding + predicate judgments
        Tiger Data (hypertable)              │ Gemini: spec drafting
              ▼
   Web UI: live verdict feed, counterexample timelines, evidence coverage
              │
   Triage agent (bounded read tools) ─► ASI:One / ACP   [secondary]

Cross-cutting: Sentry Logs + Tracing on real paths (ingest, Jev calls, exchange).
Cross-cutting: OTel dual export. Agents and exchange emit OTel spans; one exporter fans out to Sentry and to our ingest (see section 5a).
```

Workspace crates (suggested): `ledger-core` (types + adapter to Sentinel events), `ledger-ingest`, `ledger-exchange`, `ledger-agents` (simulated buyers + fault injection), `ledger-jev`, `ledger-spec` (Gemini drafting), `ledger-store` (Tiger Data), `ledger-api`, `ledger-triage`, plus the web UI.

## 3. Verified facts

**Sponsor dataset** (`CL251215/QuickBooks-Hackathon-2026`): one 26 MB tarball, `hackathon-100-participant.tar.gz`, 100 synthetic companies as YAML, one file per entity.
- Bills: `id`, `vendor_id`, `vendor_name`, `txn_date`, `due_date`, `item_lines[]` or `account_lines[]`, `total_amount`, `memo`. Bill payments: `applications[{document_id, amount}]`, `txn_date`, `memo`.
- Money is decimal strings. IDs are company-scoped; names repeat across companies.
- Scan of 39,563 bills: 0 overpaid, 0 with multiple payments, 0 paid before issue. No approvals, credits, refunds, actors or event logs. Bank feed matches the books exactly.
- Bill memos are 20 template strings; payment memos are 2. The text is mostly noise (landlord bills say "Stock received into the warehouse").
- 2,945 same-vendor, same-amount groups are recurring billing (company-001: 50 weekly $500 landlord bills). A naive duplicate detector floods on these.
- `hidden/` is simulation ground truth. Do not use it.
- No company named "Acme Manufacturing". Candidate: company-001 Hearthloaf Bakeries.
- Dataset changes must be disclosed in the submission.

**Sentinel:**
- Its control plane (`periphery`) is a "sample-only CAT-1 bootstrap, not live OTLP"; do not depend on it.
- `Event` is a protobuf-backed type with source tiers, assurance, observation time and data class. The adapter must fill these honestly. Plan: the exchange is one source, each agent's tool-call log another, so Evidence Coverage measures something real.
- Pattern catalogue (Dwyer lineage): absence, existence, universality, precedence, response, scoped global/before/after/between. Missing evidence is never a pass: verdicts are `Satisfy`, `Violate` or `Inconclusive`.
- Python Probe SDK exists; not used.

**Credentials:** `~/projects/ledger/.env` was copied from `~/projects/echo/.env` (mode 600, gitignored, values never read). It contains **only `GEMINI_API_KEY`**. No Jev key, no Tiger Data, Sentry or Fetch credentials.

## 4. Product

A live platform ingesting event streams from buying agents, each acting for one dataset company: paying that company's real bills to its real vendors, and trading on the mini exchange. Specs are Sentinel Patterns. Verdicts stream to a UI with counterexample timelines.

**Anomalies are faults in misbehaving agents**, injected by configuration and run live, not edited dataset rows:
- double-pay a settled obligation
- pay without a prior approval
- exceed a budget
- pay a vendor whose name was swapped or aliased
- follow an instruction hidden in a bill memo

**Example specs:**
- Precedence: a bill's approval precedes its payment, scoped per bill.
- Absence after: no second payment on a settled obligation unless a credit intervenes.
- Response: an order gets a fill or cancel within N ticks.

**Exchange:** small. Two assets, limit-order book, settlement as an SPL transfer on a local validator.

## 5. Jev usage

Jev (System One) gives typed `choice`, `score` or `noul` (yes/no) outputs. It is the primary-sponsor feature, so use it aggressively wherever it does something regular ML cannot (training, flexibility, speed or cost), even for small uses.

1. **Binding resolution:** are two bills (or vendor references) the same obligation? The answer merges monitor keys.
2. **Cold predicates:** yes/no questions over free text, such as "does this memo instruct a payment?" or "does this memo promise a refund?".
3. **Spec drafting check:** Gemini drafts a Pattern from a plain-English policy; Jev judges whether the draft matches the sentence. Only catalogue-valid Patterns are accepted.
4. **Evidence-coverage materiality:** is missing evidence material to this verdict?

**Contract:**
- Short, structured, provenance-carrying input only.
- Map output to `Same|Different|Unknown`, or the equivalent for each predicate, with two fixed thresholds set before looking at results. Probabilities are scores, not calibrated confidence.
- Timeouts, errors and contradictions give `Unknown`, which yields `Inconclusive`, never a pass or a violation.
- Deterministic guards run first (amounts, IDs, currency) and skip Jev when they decide.
- The model generates no explanation text; explanations come from matched evidence and reason codes.
- Store every input and output for the UI.
- **Baseline:** show exact-match and a deterministic fuzzy matcher against Jev on three controlled cases, one a negative (the recurring-bill case). Report cases, not accuracy.
- Jev's API shape is unverified. The first task is one real call, with the response saved.

## 5a. Sentry telemetry as a data source (dual export)

The buying agents and the exchange are instrumented with standard `tracing`/OTel spans (e.g. `agent.decide`, `agent.pay_bill`, `exchange.submit_order`, `exchange.settle`). One exporter fans the same spans out to **Sentry** and to **our ingest**, which normalizes them into Sentinel Events. This follows Sentinel ADR-0002: OTel is an adapter, not the substrate.

- **Verdicts come from our ingest, never from Sentry.** Sentry is a view, not the source of truth.
- **Ingest must never sample.** Set the Sentry trace sample rate to 1.0 for the demo.
- **Causality:** spans give only parent/child and timestamps. Cross-agent causal links (e.g. approval -> payment, order -> fill) are explicit span attributes carrying link IDs, which the adapter maps to Sentinel causal edges.
- **Not doing at first:** pulling spans back out of Sentry's API or webhooks (latency, auth, rate limits, dependence on Sentry uptime). Optional later extra.
- Pitch: the platform verifies agent behavior using the same trace data an engineer already has in Sentry.
- The exact `sentry` / `sentry-tracing` crate feature names (especially Logs) are unverified; check the crate docs first.

## 5b. Requirement: Sentry on the exchange

The Solana exchange **must** be instrumented with Sentry. This is a hard requirement, not a stretch goal.

- Tracing on the full order path: submit, match, fill, Solana settlement (include the tx signature as a span attribute).
- Logs on order lifecycle events, rejections and settlement failures.
- Error capture on settlement and validator failures.
- Trace context propagates from buying agent to exchange, so one trace covers decision -> order -> fill -> settlement.
- At least one real fault found and fixed through Sentry (e.g. a slow or failed settlement), shown in the demo. Do not fake an incident.

## 6. Build order

1. Workspace + event adapter. Test: a double-payment trace yields `Violate` from the real Sentinel core.
2. Exchange + buying agents + fault injection. Sentry tracing/logs on the exchange from the start (section 5b), with OTel dual export to ingest (section 5a).
3. Live ingest into Tiger Data (hypertable); verdict stream.
4. Jev and Gemini integrations.
5. UI (CLI first; the counterexample timeline is the centerpiece).
6. Triage agent + ASI:One / ACP.
7. Sentry across the remaining paths (Jev calls, ingest, API); Solana polish.

Cut order if time runs short: Solana polish, then Tiger Data, then Fetch. Keep Intuit/Jev. The exchange's Sentry instrumentation is not cut; if Solana settlement itself is stubbed, the stub is still instrumented.

## 7. Sponsors

- **Intuit / Jev (primary):** Jev does bounded semantic work in at least one core path; show a case where it changes the finding versus exact and fuzzy matching; disclose dataset modifications and Jev failures.
- **Fetch.ai:** register on Agentverse, implement ACP, make the triage agent discoverable and run its primary workflow in ASI:One. Also needs public GitHub with instructions, the ASI:One shared chat URL, Agentverse profile URL, demo video, Devpost URL, and the separate ASI:One Submission Agent flow. Docs: https://www.fetch.ai/events/hackathons/la-hacks-ai-hackathon-2026/hackpack
- **Sentry:** Logs + Tracing on Jev calls, the adapter, ingest and (required) the exchange; the same spans feed our ingest via OTel dual export; show one real failure found and fixed. Do not fake an incident.
- **MLH Tiger Data:** real hypertable for live events and a substantive time-window SQL query visible in the demo or README.
- **Gemini:** spec drafting.
- Confirm Devpost rules before assuming multi-track eligibility.

## 8. Non-goals

- No new spec DSL; use Sentinel's Patterns only.
- No claims of formally proving a model judgment.
- No real money, no real bank or QuickBooks account integration.
- No fraud claims; say "needs review".
- No agent swarm; the triage agent is a thin read layer.

## 9. Open items

- Jev key and docs (blocker).
- Tiger Data service and credentials.
- Sentry project DSN.
- Agentverse / ASI:One account and a public endpoint for the ACP agent.
- `solana-test-validator` installed?
- Sentinel `Event` field-by-field mapping from ledger events (first real code task).
- Devpost: confirm multi-track rules and that pre-existing Sentinel code is allowed.

## 10. Decisions of 2026-10-18 (grill session)

- **Demo spine:** one company (company-001), 2-3 buying agents, a Rust exchange with real Solana settlement (local `solana-test-validator`, installed), live fault injection, real Sentinel verdicts, Jev bindings, web UI.
- **Cut:** ASI:One / Fetch / triage agent (sections 2, 6 step 6, 7 are void for Fetch).
- **Tiger Data:** replaced by a TimescaleDB container (hypertable + time-window SQL). Needs OrbStack/Docker running.
- **Jev key:** `TYPESAFE_API_KEY` in `.env`; first Jev task verifies it with a saved fixture.
- **Jev monitor GC (new use #5):** retire per-binding monitors via Sentinel `RetirementSignal`. Deterministic close first; Jev only for ambiguous closes; unknown/timeout/low score keeps the monitor alive (fail toward keeping).
- **Sentry:** `SENTRY_DSN` in `.env`; sample rate 1.0, `send_default_pii` false.
- **UI (web, axum SSE + Vite/React), four views:** metrics dashboard; live trace ingest; live spec authoring; live verdict monitoring with counterexample timeline. The exact/fuzzy/Jev comparison lives in the verdict detail and README, not its own screen.
- Work is tracked as GitHub issues on `arniber21/ledger`.
