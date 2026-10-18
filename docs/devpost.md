# Devpost checklist

Checked on 2026-10-18 against the public pages:

- https://la-ai-hackathon-2026.devpost.com/
- https://la-ai-hackathon-2026.devpost.com/rules

Deadline on those pages: Oct 18, 2026, 12:00pm PDT. The rules page still contains an older line, "Submission time ends at 8am (PDT), Sunday, April 26th." The dated schedule on the hackathon page is Oct 18, 2026, 12:00pm PDT. Use the dated schedule, and re-check the submit form before uploading.

## What the pages require

- Project description: what it does, the problem, which track it falls under, and how it applies to the company challenges submitted to.
- Public code repository, documented.
- Demo video, 2–3 minutes.
- A Devpost page with the summary and technologies.
- In-person attendance for judging.
- Team of at most 4 students. Work is done at the hackathon.

## Multi-track

The overview says the project must "align with one of these tracks." The prize list has three named track prizes: Model Learning, Beyond the Digital, and Applied Intelligence. The description prompt asks "which track it falls under" (singular).

The same prompt asks "How does it apply to the company challenges you submitted to?" The word "challenges" is plural. Sponsor prizes are listed separately from those three tracks, including Intuit, Sentry, Gemini API, Tiger Data, Solana, and Fetch.ai.

The public rules do not say a project may enter every sponsor prize at once, and they do not say a project is limited to a single sponsor prize. The submit form is behind login, so the checkboxes on that form are the binding list. Pick one thematic track. Select company challenges only where the form offers them and the project actually does that work. Do not assume one submission is eligible for every sponsor prize.

Fetch.ai (Agentverse, ACP, ASI:One) was cut from this project. Do not enter that prize.

## Pre-existing Sentinel code

Rule 3.1: "Pre-existing work may be used provided that it is open source."

Rule 3.2: adding features to an existing project is allowed, the submission must state which features were developed during the hackathon, and judges consider only that new functionality.

`vendor/sentinel` is pre-existing. Its workspace `Cargo.toml` says `license = "Proprietary"`, `authors = ["Emet Labs"]`, and `publish = false`. That tree is not declared open source. Under the published rule, pre-existing Sentinel code is **not confirmed as allowed**. Disclose it. Do not describe it as open source, and do not present it as work from this hackathon.

Hackathon work to name in the submission: the ledger crates and `web/` (dataset loader, event adapter, exchange and local Solana settlement, buying agents and live faults, ingest, Timescale storage and the window query, Jev and Gemini calls, API, and UI).

## What not to claim

- Do not claim a real Sentry incident was found and fixed. Tracing and logs on the exchange are in the repo. A recorded incident is separate and is not produced by `just demo`.
- Do not claim Jev results that were not returned by a real call or saved from one.
- Do not say "fraud". Say "needs review".
- The local database is TimescaleDB (`timescale/timescaledb`) with a hypertable and `time_bucket`. Say that. Do not describe it as a Tiger Cloud account.
- The sponsor dataset is not modified. `hidden/` is not read. `data/` is gitignored because the dataset has no declared license.
