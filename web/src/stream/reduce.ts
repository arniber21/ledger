import {
  TEN_MINUTES_MS,
  type CounterPatch,
  type MetricsSnapshot,
  type BindingComparison,
  type BindingLine,
  type BindingOutcome,
  type RatePoint,
  type SavedView,
  type StreamEvent,
  type RerankOutcome,
  type RerankRecord,
  type RerankScore,
  type TimelineStep,
  type VerdictInput,
  type VerdictItem,
  type VerdictOutcome,
} from "./types";

const OUTCOMES: readonly VerdictOutcome[] = ["Satisfy", "Violate", "Inconclusive"];

export function isOutcome(value: unknown): value is VerdictOutcome {
  return typeof value === "string" && OUTCOMES.includes(value as VerdictOutcome);
}

export function isSavedView(value: unknown): value is SavedView {
  return (
    value === "assigned" ||
    value === "violations" ||
    value === "unknowns" ||
    value === "over-1000"
  );
}

export function outcomeLabel(outcome: VerdictOutcome): string {
  switch (outcome) {
    case "Violate":
      return "Needs review";
    case "Inconclusive":
      return "Inconclusive";
    case "Satisfy":
      return "Satisfied";
  }
}

/** Violate and inconclusive need a person. Satisfied does not. */
export function needsAttention(verdict: VerdictItem): boolean {
  return verdict.outcome === "Violate" || verdict.outcome === "Inconclusive";
}

export function matchesSavedView(verdict: VerdictItem, view: SavedView): boolean {
  switch (view) {
    case "assigned":
      return verdict.assignedToMe;
    case "violations":
      return verdict.outcome === "Violate";
    case "unknowns":
      return verdict.jevUnknown;
    case "over-1000":
      return verdict.amountCents != null && verdict.amountCents >= 100_000;
  }
}

export function countSavedView(verdicts: readonly VerdictItem[], view: SavedView): number {
  return verdicts.filter((verdict) => matchesSavedView(verdict, view)).length;
}

function finiteNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function trimWindow<T extends { t: number }>(points: readonly T[], latest: number): T[] {
  const cutoff = latest - TEN_MINUTES_MS;
  return points.filter((point) => point.t >= cutoff);
}

function normalizeVerdict(input: VerdictInput): VerdictItem | null {
  if (typeof input.id !== "string" || input.id.length === 0) return null;
  if (!isOutcome(input.outcome)) return null;
  if (typeof input.title !== "string" || input.title.length === 0) return null;
  const amount = input.amountCents;
  const comparison = readComparison(input.comparison);
  return {
    id: input.id,
    outcome: input.outcome,
    title: input.title,
    evidence: typeof input.evidence === "string" ? input.evidence : "",
    specId: typeof input.specId === "string" ? input.specId : "",
    reasonCode: typeof input.reasonCode === "string" ? input.reasonCode : null,
    amountCents: typeof amount === "number" && Number.isFinite(amount) ? amount : null,
    assignedToMe: input.assignedToMe === true,
    jevUnknown: input.jevUnknown === true || comparison?.jev?.outcome === "Unknown",
    recordedAt: typeof input.recordedAt === "string" ? input.recordedAt : "",
    binding: typeof input.binding === "string" ? input.binding : "",
    timeline: readTimeline(input.timeline),
    rest: readTimeline(input.rest),
    rank: input.rank === "ranked" || input.rank === "unknown" ? input.rank : null,
    rerank: readRerank(input.rerank),
    comparison,
  };
}

const BINDING_OUTCOMES: readonly BindingOutcome[] = ["Same", "Different", "Unknown"];

const RERANK_OUTCOMES: readonly RerankOutcome[] = ["Relevant", "NotRelevant", "Unknown"];

function readRerank(value: unknown): RerankRecord | null {
  if (!isRecord(value)) return null;
  const scores = readScores(value.scores);
  const request = typeof value.request === "string" ? value.request : "";
  const response = typeof value.response === "string" ? value.response : "";
  const failure = typeof value.failure === "string" && value.failure.length > 0 ? value.failure : null;
  if (scores.length === 0 && request.length === 0 && response.length === 0 && !failure) return null;
  return { request, response, failure, scores };
}

function readScores(value: unknown): RerankScore[] {
  if (!Array.isArray(value)) return [];
  const scores: RerankScore[] = [];
  for (const entry of value) {
    if (!isRecord(entry)) continue;
    const eventId = typeof entry.eventId === "string" ? entry.eventId : "";
    const outcome = entry.outcome;
    if (eventId.length === 0 || !isRerankOutcome(outcome)) continue;
    scores.push({ eventId, outcome });
  }
  return scores;
}

function isRerankOutcome(value: unknown): value is RerankOutcome {
  return typeof value === "string" && RERANK_OUTCOMES.includes(value as RerankOutcome);
}

function readTimeline(value: unknown): TimelineStep[] {
  if (!Array.isArray(value)) return [];
  const steps: TimelineStep[] = [];
  for (const entry of value) {
    if (typeof entry !== "object" || entry === null) continue;
    const step = entry as Record<string, unknown>;
    if (typeof step.eventId !== "string" || step.eventId.length === 0) continue;
    const source = typeof step.source === "string" ? step.source : "";
    const kind = typeof step.kind === "string" ? step.kind : "";
    const role = typeof step.role === "string" ? step.role : "";
    if (source.length === 0 && kind.length === 0 && role.length === 0) continue;
    steps.push({
      eventId: step.eventId,
      observedAt: typeof step.observedAt === "string" ? step.observedAt : "",
      source,
      kind,
      role,
    });
  }
  return steps;
}

function readComparison(value: unknown): BindingComparison | null {
  if (!isRecord(value)) return null;
  const exact = readBindingLine(value.exact);
  const fuzzy = readBindingLine(value.fuzzy);
  let jev = readBindingLine(value.jev);
  if (!jev && isRecord(value.jevFault) && typeof value.jevFault.kind === "string" && value.jevFault.kind.length > 0) {
    jev = { outcome: "Unknown", reasonCode: value.jevFault.kind };
  }
  if (!exact && !fuzzy && !jev) return null;
  return { exact, fuzzy, jev };
}

function readBindingLine(value: unknown): BindingLine | null {
  if (typeof value === "string") {
    return isBindingOutcome(value) ? { outcome: value, reasonCode: null } : null;
  }
  if (!isRecord(value)) return null;
  const outcome = typeof value.outcome === "string" ? value.outcome : value.verdict;
  if (!isBindingOutcome(outcome)) return null;
  const reasonCode = typeof value.reasonCode === "string" && value.reasonCode.length > 0 ? value.reasonCode : null;
  return { outcome, reasonCode };
}

function isBindingOutcome(value: unknown): value is BindingOutcome {
  return typeof value === "string" && BINDING_OUTCOMES.includes(value as BindingOutcome);
}

function applyCounters(snapshot: MetricsSnapshot, counters: CounterPatch): MetricsSnapshot {
  const next = { ...snapshot };
  const fields = [
    "eventsIngested",
    "eventsPerSecond",
    "sourceCount",
    "liveMonitors",
    "retiredByGc",
    "jevClosedAmbiguous",
    "settledOnSolana",
    "settlementsFailed",
    "jevJudgments",
    "jevUnknown",
  ] as const;
  for (const field of fields) {
    const value = finiteNumber(counters[field]);
    if (value !== undefined) next[field] = value;
  }
  return next;
}

function rateSources(point: RatePoint): number {
  return [point.agentLog, point.exchange, point.solana].filter((value) => value > 0).length;
}

/**
 * Apply one stream message. Returns the same snapshot when the message is
 * unusable, so a bad event never becomes a pass.
 */
export function applyStreamEvent(snapshot: MetricsSnapshot, event: StreamEvent): MetricsSnapshot {
  switch (event.kind) {
    case "hello":
      return { ...event.snapshot, connection: "live" };
    case "verdict": {
      const verdict = normalizeVerdict(event.verdict);
      if (!verdict) return snapshot;
      const without = snapshot.verdicts.filter((item) => item.id !== verdict.id);
      return { ...snapshot, connection: "live", verdicts: [verdict, ...without] };
    }
    case "counters":
      return { ...applyCounters(snapshot, event.counters), connection: "live" };
    case "rates": {
      const point = event.point;
      if (!Number.isFinite(point.t)) return snapshot;
      const rates = trimWindow([...snapshot.rates, point], point.t);
      return {
        ...snapshot,
        connection: "live",
        rates,
        eventsPerSecond: point.agentLog + point.exchange + point.solana,
        sourceCount: rateSources(point),
      };
    }
    case "attention": {
      const point = event.point;
      if (!Number.isFinite(point.t)) return snapshot;
      return {
        ...snapshot,
        connection: "live",
        attention: trimWindow([...snapshot.attention, point], point.t),
      };
    }
    case "monitors": {
      const point = event.point;
      if (!Number.isFinite(point.t)) return snapshot;
      return {
        ...snapshot,
        connection: "live",
        monitors: trimWindow([...snapshot.monitors, point], point.t),
        liveMonitors: point.withGc,
      };
    }
    default:
      return snapshot;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function readPointTime(value: unknown): number | null {
  if (!isRecord(value)) return null;
  const t = finiteNumber(value.t);
  return t === undefined ? null : t;
}

/**
 * Parse one SSE data payload. Returns null when the JSON is not a known event
 * or an outcome is outside Satisfy / Violate / Inconclusive.
 */
export function parseStreamData(raw: string): StreamEvent | null {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!isRecord(value) || typeof value.kind !== "string") return null;

  switch (value.kind) {
    case "verdict": {
      if (!isRecord(value.verdict)) return null;
      const verdict = value.verdict;
      if (typeof verdict.id !== "string" || typeof verdict.title !== "string") return null;
      if (!isOutcome(verdict.outcome)) return null;
      return { kind: "verdict", verdict: verdict as VerdictInput };
    }
    case "counters": {
      if (!isRecord(value.counters)) return null;
      return { kind: "counters", counters: value.counters as CounterPatch };
    }
    case "rates":
    case "attention":
    case "monitors": {
      if (readPointTime(value.point) === null) return null;
      return value as StreamEvent;
    }
    case "hello": {
      if (!isRecord(value.snapshot) || !Array.isArray(value.snapshot.verdicts)) return null;
      const verdicts: VerdictItem[] = [];
      for (const item of value.snapshot.verdicts) {
        if (!isRecord(item)) return null;
        const verdict = normalizeVerdict(item as VerdictInput);
        if (!verdict) return null;
        verdicts.push(verdict);
      }
      return {
        kind: "hello",
        snapshot: { ...(value.snapshot as MetricsSnapshot), verdicts },
      };
    }
    default:
      return null;
  }
}

export function attentionCount(verdicts: readonly VerdictItem[]): number {
  return verdicts.filter(needsAttention).length;
}
