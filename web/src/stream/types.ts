/**
 * Dashboard snapshot and the SSE messages ledger-api (#9) can send later.
 *
 * Outcomes match `VerdictOutcome` in ledger-store: Satisfy, Violate,
 * Inconclusive. Missing evidence stays Inconclusive. The UI says
 * "needs review" for Violate.
 */

export type VerdictOutcome = "Satisfy" | "Violate" | "Inconclusive";

export type Connection = "live" | "offline";

export type SavedView = "assigned" | "violations" | "unknowns" | "over-1000";

export type InboxView = "board" | "list" | "split";

/** One evidence step from `GET /v1/verdicts/stream`. */
export type TimelineStep = {
  eventId: string;
  observedAt: string;
  source: string;
  kind: string;
  role: string;
};

/** One verdict the dashboard can show. Evidence only; no model prose. */
export type VerdictItem = {
  id: string;
  outcome: VerdictOutcome;
  title: string;
  evidence: string;
  specId: string;
  reasonCode: string | null;
  amountCents: number | null;
  assignedToMe: boolean;
  jevUnknown: boolean;
  recordedAt: string;
  binding?: string;
  timeline?: TimelineStep[];
};

/** One bucket from a 10-minute events-per-second series, by source. */
export type RatePoint = {
  /** Unix milliseconds at the start of the bucket. */
  t: number;
  agentLog: number;
  exchange: number;
  solana: number;
};

/** Violate and inconclusive counts for one minute. Satisfied is omitted. */
export type AttentionPoint = {
  t: number;
  violate: number;
  inconclusive: number;
};

/** Live monitor count, and the count if retirement had not run. */
export type MonitorPoint = {
  t: number;
  withGc: number;
  withoutGc: number;
};

export type MetricsSnapshot = {
  connection: Connection;
  eventsIngested: number;
  eventsPerSecond: number;
  sourceCount: number;
  liveMonitors: number;
  retiredByGc: number;
  jevClosedAmbiguous: number;
  settledOnSolana: number;
  settlementsFailed: number;
  jevJudgments: number;
  jevUnknown: number;
  rates: RatePoint[];
  attention: AttentionPoint[];
  monitors: MonitorPoint[];
  verdicts: VerdictItem[];
};

export type CounterPatch = {
  eventsIngested?: number;
  eventsPerSecond?: number;
  sourceCount?: number;
  liveMonitors?: number;
  retiredByGc?: number;
  jevClosedAmbiguous?: number;
  settledOnSolana?: number;
  settlementsFailed?: number;
  jevJudgments?: number;
  jevUnknown?: number;
};

/**
 * One JSON object in an SSE `data:` line.
 *
 * `hello` replaces the snapshot. Later messages patch it. The hook ignores
 * anything that does not match.
 */
export type StreamEvent =
  | { kind: "hello"; snapshot: MetricsSnapshot }
  | { kind: "verdict"; verdict: VerdictInput }
  | { kind: "counters"; counters: CounterPatch }
  | { kind: "rates"; point: RatePoint }
  | { kind: "attention"; point: AttentionPoint }
  | { kind: "monitors"; point: MonitorPoint };

/** Fields #9 must send. The rest default so a partial verdict still applies. */
export type VerdictInput = {
  id: string;
  outcome: VerdictOutcome;
  title: string;
  evidence?: string;
  specId?: string;
  reasonCode?: string | null;
  amountCents?: number | null;
  assignedToMe?: boolean;
  jevUnknown?: boolean;
  recordedAt?: string;
  binding?: string;
  timeline?: TimelineStep[];
};

export const SAVED_VIEWS: { id: SavedView; label: string }[] = [
  { id: "assigned", label: "Assigned to me" },
  { id: "violations", label: "Violations only" },
  { id: "unknowns", label: "Jev unknowns" },
  { id: "over-1000", label: "Over $1,000" },
];

export const TEN_MINUTES_MS = 10 * 60 * 1000;

export function emptySnapshot(): MetricsSnapshot {
  return {
    connection: "offline",
    eventsIngested: 0,
    eventsPerSecond: 0,
    sourceCount: 0,
    liveMonitors: 0,
    retiredByGc: 0,
    jevClosedAmbiguous: 0,
    settledOnSolana: 0,
    settlementsFailed: 0,
    jevJudgments: 0,
    jevUnknown: 0,
    rates: [],
    attention: [],
    monitors: [],
    verdicts: [],
  };
}
