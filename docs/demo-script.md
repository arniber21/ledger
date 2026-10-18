# Demo script and playbook

For an audience that does not know runtime verification. Read the **Say** lines; do the **Do** lines. Everything is simulated: say so once, early.

## The one-sentence pitch

"AI agents are starting to pay bills and trade for companies. Ledger Sentinel is a smoke detector for that: you write the rules in plain English, it watches every action as it happens, and when a rule is broken it shows you the evidence, step by step."

If someone asks "what is runtime verification": "Tests check your code before you ship. This checks what the agents actually *did*, live, against rules you wrote."

## Words to use, words to avoid

| Say | Not |
|---|---|
| Needs Review | violation, fraud (we never say fraud; it is a simulation and we do not judge intent) |
| Satisfied | pass |
| Inconclusive: "we can't tell yet, and we never count missing evidence as OK" | unknown, undefined |
| rule / policy | specification, monitor, temporal logic |
| the evidence trail / timeline | counterexample trace |
| a small AI judge (Jev) for fuzzy questions | System One model |

## Before you go on (10 minutes before)

1. `just reset` then `just demo` (about 20 seconds). This loads clean, coherent scenes: a clean run, a double payment, a hidden instruction in a memo, a vendor-name mismatch.
2. `just up` (starts the validator, database, API, agents and UI). Wait for "Ctrl-C stops the processes".
3. Open http://127.0.0.1:5173 full screen, dark-mode off, browser zoom 100-125%.
4. Check: `lsof -nP -iTCP:8080 -sTCP:LISTEN` shows only your API. Other worktree stacks on 8080 or 5173 will break the page (it shows "Offline" and zeros).
5. Open a second terminal at the repo root for `just fault double-pay` and `just window-query`.
6. Have a screen recording of a good run ready as a backup.

Known caveats (do not claim past these):
- Scene data is stamped on a fixed old clock, so ages on those cards show a dash. That is expected.
- Settlement spans from the live desk are marked `simulated`. Say "simulated settlement".
- Sentry receives our traces, logs and errors. There is no recorded real incident. Do not say there is one.
- Live spec authoring can end "Inconclusive" if the Jev check is unsure. That is the design (uncertain means do not arm). Use the replayed example, not a surprise sentence, on stage.

## The 3-minute version

**0:00 Dashboard (set the scene)**
Do: land on Dashboard.
Say: "This is a small bakery company, Hearthloaf. AI agents pay its real bills and buy flour on an exchange. Everything is simulated, no real money. Top row is what is happening right now: events coming in, how many rules are watching, payments settled on a Solana test chain, and judgments from a small AI model."

**0:30 Inbox (the 'so what')**
Do: press `g` then `i`, or click Inbox. Use J/K to move down the cards.
Say: "Every rule that fails, and every case we can't be sure about, lands here like an email inbox. Three states: Needs Review, Inconclusive, Satisfied. Notice we do not call anything fraud. We say needs review."

**1:00 Open the double-payment**
Do: select the Needs Review card for the second payment on a bill, press Enter (or open it in Verdicts).
Say: "The rule was 'a bill is paid once'. Here is the evidence trail: the first payment, the settlement, then a second payment on the same bill. That is the counterexample, three steps, every one clickable. Nobody had to read logs."

**1:45 Why it is more than a log search**
Say: "The rules are written in plain English and compiled into monitors. There is one live monitor per open bill, and when a bill is done we retire it, so it stays small. That is the 'Live monitors' number on the dashboard."

**2:15 The AI-judge moment**
Do: open the Inconclusive vendor-alias card (exact match Different, fuzzy match Different, Jev Same).
Say: "Sometimes code can't tell if 'Northwind Freight' and 'North-wind Freight Inc' are the same vendor. A small AI model, Jev, answers that one narrow question with a probability. Under 0.80 we say unknown, and unknown means inconclusive, never a pass. The model never writes explanations; the evidence is facts."

**2:45 Close**
Say: "Same idea works for any agent that touches money: write the rule, watch the stream, get evidence. Built in Rust on a runtime-verification engine, Timescale for time-series, Solana for settlement, Sentry for telemetry, Gemini to draft rules from sentences."

## The 5-minute version (add these)

**Inject a live fault (after the double-pay card)**
Do: press Cmd+K, pick a fault (double-pay), confirm. Or in the second terminal: `just fault double-pay`. Leave the Inbox open.
Say: "Now I'll make the agent misbehave, live." (The desk runs a full cycle, so it can take 10-60 seconds. Talk through the Traces screen while it runs.)
If nothing new lands within a minute, say "that cycle is still running" and go back to the prepared double-pay card. Do not stall on stage.

**Traces**
Do: Traces screen. Point at spans appearing: approve, pay, settle, credit.
Say: "Every action emits a span. The same spans go to Sentry and to our own store, which is how rules see them."

**Specs (authoring)**
Do: Specs, new spec, use the replayed example sentence.
Say: "A person writes a policy sentence. Gemini drafts a rule, we check it against the catalogue of supported patterns, and Jev checks the draft really means the sentence. Only then does it arm."

**Timescale**
Do: second terminal, `just window-query`.
Say: "Because events are in a time-series database, 'how many events per minute for the last hour' is one query."

## Q&A cheat sheet

- **Is this real money / real fraud detection?** No, simulated. We flag "needs review", never intent.
- **Why not just use logs and alerts?** Alerts match patterns you thought of. These are explicit rules that say what must hold and show the exact steps when they don't.
- **What happens when the AI judge is unsure?** Inconclusive. Deterministic checks run first; uncertain answers never count as a pass.
- **Does the AI explain violations?** No. Models only return typed judgments; the evidence trail is built from recorded events.
- **What if the evidence never arrives?** Inconclusive, not a pass.
- **How does it scale?** Monitors are per bill and retired when the bill closes; the dashboard shows live monitors with and without cleanup.
- **Where is Sentry?** It receives the agents' and exchange's traces, logs and errors; verdicts link to the trace.
- **Is the Solana part real?** A local test validator with one token transfer per fill. Say "local test chain".

## If something breaks

| Symptom | Do |
|---|---|
| UI says Offline, all zeros | Another process owns :8080 or :5173. `lsof -nP -iTCP:8080 -sTCP:LISTEN`, stop the stranger, `just up` again. |
| Live fault never shows a verdict | Say the cycle is slow; open the prepared double-pay card from `just demo`. |
| Spec authoring says Inconclusive | Explain it is the safe default and move on. |
| Everything is on fire | Play the screen recording and narrate the same lines. |
