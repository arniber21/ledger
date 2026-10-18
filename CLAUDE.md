# CLAUDE.md

Ledger Sentinel: live runtime verification of simulated financial agents. Read `SPEC.md` (esp. section 10) and the GitHub issue you were given; issues on `arniber21/ledger` are the source of truth for scope, "Blocked by" lines are real.

## Rules
- Rust workspace under `crates/`; web UI under `web/` (Vite + React + TanStack Router/Query/Table, bun). Use `just` for tasks (`just check`, `just test`, `just lint`).
- Sentinel (`/Users/arnabghosh/Desktop/emet/sentinel`) is pre-existing, proprietary, consumed by path. Never copy it into this repo; never edit it. Use the incremental `Evaluator::step` + `seal` path.
- Secrets live in gitignored `.env` (`GEMINI_API_KEY`, `TYPESAFE_API_KEY` = Jev, `SENTRY_DSN`). Never print, log or commit values.
- Never read the dataset's `hidden/` directory.
- Everything is simulated. Say "needs review", never "fraud".
- Missing or failed evidence is `Inconclusive`, never a pass. Jev timeout/error => `Unknown`. Models generate no explanation text.
- Do not fake Sentry incidents or Jev results.
- Run `just check` and `just test` before finishing. One issue per branch/PR; reference it (`Closes #N`). Commit messages end with the Co-Authored-By line.
- Prefer cloning/adapting small open-source examples (exchange, SPL transfer) over inventing, and note the source in the file header.
- Tools: Docker (OrbStack), `solana-test-validator`/`surfpool`, bun, just. Solana CLI is on PATH via the Justfile.
- Dataset: run `just data` (downloads from HuggingFace `CL251215/QuickBooks-Hackathon-2026`, extracts company-001 only into gitignored `data/`; not committed because it has no declared license and the repo is public). Files: `data/hackathon-100/company-001/rwm-v4/world-*/{bills,vendors,bill_payments,company,terms,accounts}.yaml`, one YAML per entity type; read `SCHEMA.md` there first.
