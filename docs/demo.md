# Demo script

Run this from the repo root. `just demo` starts TimescaleDB, the local Solana validator, the API, the buying agents, and the web UI. Ctrl-C stops the processes that command started.

A real Sentry incident is not part of this script. Do not invent one, and do not describe a fabricated incident as something this run produced.

## Before you start

`.env` is gitignored and stays in the repo root. It needs `GEMINI_API_KEY` and `TYPESAFE_API_KEY`. `SENTRY_DSN` is optional for startup: without it the stack still runs, and traces are not sent to Sentry. The script never prints these values.

Docker (OrbStack) must be running. `solana-test-validator`, `bun`, Rust, and `just` must be on `PATH`. The Justfile adds the Solana CLI path when you use `just`.

## Run of show

1. `just demo`
   - If `data/hackathon-100/company-001` is missing, the command downloads company-001 first. It does not extract or read `hidden/`.
   - When the command is ready it prints the UI URL, `http://127.0.0.1:5173`.
   - Leave this terminal running.
2. Open the UI.
   - Inbox: `http://127.0.0.1:5173/inbox`
   - Dashboard: `http://127.0.0.1:5173/`
   - Traces: `http://127.0.0.1:5173/traces`
   - Specs: `http://127.0.0.1:5173/specs`
   - Verdicts: `http://127.0.0.1:5173/verdicts`
3. In a second terminal, at the same repo root: `just fault double-pay`
   - The desk pays a settled bill again and marks that payment **needs review**.
   - Other live faults: `no-approval`, `exceed-budget`, `swapped-vendor`, `hidden-memo`.
   - Say "needs review". Do not say "fraud".
4. Open the verdict in `/verdicts`. The counterexample timeline is the evidence for that verdict. Missing evidence is `Inconclusive`, not a pass.
5. `just window-query`
   - This prints per-minute event counts for the last hour from the Timescale hypertable. The SQL is in the README.
6. Jev, if you show a binding or a memo judgment:
   - A timeout, transport failure, HTTP error, or unreadable body is `Unknown`.
   - A score under the fixed 0.80 threshold, or a contradiction, is also `Unknown`.
   - `Unknown` is `Inconclusive`. It does not merge monitor keys and it is not a pass.
   - Use a real call or an already saved fixture. Do not invent a Jev result.

## Not this script

Finding a genuine slow or failed settlement in Sentry, fixing it, and recording that for the demo is separate work. If `SENTRY_DSN` is set, exchange and agent spans are exported. That export is not a recorded incident.
