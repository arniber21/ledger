/**
 * Trace screen model.
 *
 * Rows match the `events` hypertable: `id`, `observed_at`, `source`, `kind`,
 * `data_class`, `payload`. Ingest writes `span`, `span_id`, `parent_span_id`,
 * `started_at`, `closed_at`, `attributes`, `causal_links`, `adapted`, and
 * sometimes `sentinel`, `evidence`, and `reason`. There is no trace-id column.
 * A trace is the spans connected by `parent_span_id`.
 */

import type { Connection } from "../stream/types";

export type CausalLink = {
  attribute: string;
  target: string;
  resolved: boolean;
};

export type SentinelSummary = {
  eventId?: string;
  kind?: string;
  source?: string;
  tier?: string;
  dataClass?: string;
  predecessors: string[];
};

export type StoredPayload = {
  span?: string;
  spanId?: string;
  parentSpanId?: string;
  startedAt?: string;
  closedAt?: string;
  attributes: Record<string, string>;
  causalLinks: CausalLink[];
  adapted?: boolean;
  evidence?: string;
  reason?: string;
  sentinel?: SentinelSummary;
};

/** One hypertable row. Payload keys are the ones ingest stores. */
export type StoredEvent = {
  id: string;
  observedAt: string;
  source: string;
  kind: string;
  dataClass: string;
  payload: StoredPayload;
};

export type Trace = {
  id: string;
  spans: StoredEvent[];
};

export type TracesSnapshot = {
  connection: Connection;
  events: StoredEvent[];
};

export type TracesMessage =
  | { kind: "hello"; events: StoredEvent[] }
  | { kind: "span"; event: StoredEvent }
  | { kind: "offline" };

const CAP = 500;

const SPAN_TITLE: Record<string, string> = {
  "agent.decide": "Decision",
  "agent.approve": "Approval",
  "agent.pay_bill": "Bill payment",
  "agent.settle": "Settlement",
  "exchange.submit_order": "Order submitted",
  "exchange.match": "Order matched",
  "exchange.fill": "Order filled",
  "exchange.settle": "Fill settled",
  "ledger.agent.pay_bill": "Bill payment",
  "ledger.exchange.payment": "Extra payment",
  "ledger.exchange.settled": "Obligation settled",
  "ledger.exchange.credit": "Credit",
};

/** Review reasons from `Fault::review_reason`. */
const REVIEW_REASON: Record<string, string> = {
  "double-pay": "second payment on a settled obligation",
  "no-approval": "payment without approval",
  "exceed-budget": "payment exceeds budget",
  "swapped-vendor": "payment vendor was swapped",
  "hidden-memo": "agent followed an instruction in a bill memo",
};

export function emptyTraces(): TracesSnapshot {
  return { connection: "offline", events: [] };
}

export function parseTracesData(data: string): TracesMessage | null {
  let value: unknown;
  try {
    value = JSON.parse(data);
  } catch {
    return null;
  }
  if (!isRecord(value) || typeof value.kind !== "string") return null;
  if (value.kind === "offline") return { kind: "offline" };
  if (value.kind === "hello") return { kind: "hello", events: parseEventList(value.events) };
  if (value.kind === "span") {
    const event = parseStoredEvent(value.event);
    return event ? { kind: "span", event } : null;
  }
  return null;
}

export function parseEventList(value: unknown): StoredEvent[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((item) => {
    const event = parseStoredEvent(item);
    return event ? [event] : [];
  });
}

export function parseStoredEvent(value: unknown): StoredEvent | null {
  if (!isRecord(value)) return null;
  if (
    typeof value.id !== "string" ||
    typeof value.observed_at !== "string" ||
    typeof value.source !== "string" ||
    typeof value.kind !== "string" ||
    typeof value.data_class !== "string"
  ) {
    return null;
  }
  return {
    id: value.id,
    observedAt: value.observed_at,
    source: value.source,
    kind: value.kind,
    dataClass: value.data_class,
    payload: parsePayload(value.payload),
  };
}

export function applyTracesEvent(snapshot: TracesSnapshot, message: TracesMessage): TracesSnapshot {
  if (message.kind === "offline") return { ...snapshot, connection: "offline" };
  const incoming = message.kind === "hello" ? message.events : [message.event];
  const events = dedupe([...snapshot.events, ...incoming]).sort(byTime).slice(-CAP);
  return { connection: "live", events };
}

/** Groups rows into traces by `payload.parent_span_id` → `payload.span_id`. */
export function groupTraces(events: StoredEvent[]): Trace[] {
  const bySpan = new Map<string, StoredEvent[]>();
  for (const event of events) {
    const spanId = event.payload.spanId;
    if (!spanId) continue;
    const list = bySpan.get(spanId) ?? [];
    list.push(event);
    bySpan.set(spanId, list);
  }

  const parent = new Map<string, string>();
  for (const event of events) {
    const parentSpan = event.payload.parentSpanId;
    if (!parentSpan) continue;
    const candidates = bySpan.get(parentSpan);
    if (!candidates || candidates.length === 0) continue;
    const observed = Date.parse(event.observedAt);
    const earlier = candidates.filter((item) => Date.parse(item.observedAt) <= observed);
    const choice = earlier.at(-1) ?? candidates[0];
    if (!choice || choice.id === event.id) continue;
    parent.set(event.id, choice.id);
  }

  const groups = new Map<string, StoredEvent[]>();
  for (const event of events) {
    const key = rootOf(event.id, parent);
    const list = groups.get(key) ?? [];
    list.push(event);
    groups.set(key, list);
  }

  return [...groups.entries()].map(([id, spans]) => ({
    id,
    spans: [...spans].sort(byTime),
  }));
}

export function traceFor(traces: Trace[], id: string | undefined): Trace | undefined {
  if (!id) return undefined;
  return traces.find((trace) => trace.id === id || trace.spans.some((span) => span.id === id));
}

export function spanName(event: StoredEvent): string {
  return event.payload.span ?? event.kind;
}

export function evidenceSentence(event: StoredEvent): string {
  const attributes = event.payload.attributes;
  const fault = attributes["fault.name"];
  if (fault) {
    const reason = REVIEW_REASON[fault];
    return reason ? `Needs review. ${capitalize(reason)}` : "Needs review.";
  }
  if (attributes["needs_review"] === "true") return "Needs review.";
  if (event.payload.evidence === "inconclusive") {
    const reason = event.payload.reason?.trim();
    return reason ? `Inconclusive. ${capitalize(reason)}` : "Inconclusive. Evidence is missing.";
  }
  return factual(spanName(event), attributes, event.source);
}

export function traceTitle(trace: Trace): string {
  const flagged = [...trace.spans].reverse().find((span) => {
    const attributes = span.payload.attributes;
    return Boolean(attributes["fault.name"]) || attributes["needs_review"] === "true";
  });
  if (flagged) return "Needs review";
  const root = trace.spans.find((span) => span.id === trace.id) ?? trace.spans[0];
  if (!root) return "Trace";
  return SPAN_TITLE[spanName(root)] ?? spanName(root);
}

export function traceDescription(trace: Trace): string {
  const root = trace.spans.find((span) => span.id === trace.id) ?? trace.spans[0];
  if (!root) return "No spans on this trace.";
  return evidenceSentence(root);
}

export function agentOf(spans: StoredEvent[]): string {
  for (const span of spans) {
    const agent = span.payload.attributes["agent.id"];
    if (agent) return agent;
  }
  const source = spans[0]?.source ?? "";
  if (source.startsWith("agent.")) return source.slice("agent.".length);
  return source || "Unknown source";
}

export function companyOf(spans: StoredEvent[]): string {
  for (const span of spans) {
    const company = span.payload.attributes["company.name"];
    if (company) return company;
  }
  return "Hearthloaf Bakeries";
}

export function tiersOf(spans: StoredEvent[]): string {
  const tiers = new Set<string>();
  for (const span of spans) {
    const tier = span.payload.sentinel?.tier;
    if (tier) tiers.add(tier);
  }
  if (tiers.size === 0) return "Not adapted";
  return [...tiers].join(", ");
}

export function sourcesOf(spans: StoredEvent[]): string {
  const sources = new Set(spans.map((span) => span.source).filter((source) => source.length > 0));
  if (sources.size === 0) return "None";
  return [...sources].join(", ");
}

export function traceDuration(spans: StoredEvent[]): string | null {
  let start = Number.POSITIVE_INFINITY;
  let end = Number.NEGATIVE_INFINITY;
  for (const span of spans) {
    const opened = Date.parse(span.payload.startedAt ?? "");
    const closed = Date.parse(span.payload.closedAt ?? span.observedAt);
    if (!Number.isNaN(opened)) start = Math.min(start, opened);
    if (!Number.isNaN(closed)) end = Math.max(end, closed);
  }
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) return null;
  const ms = Math.round(end - start);
  if (ms < 1000) return `${ms} ms`;
  const seconds = ms / 1000;
  return `${seconds.toFixed(1)} s`;
}

export function signatureOf(spans: StoredEvent[]): string | null {
  for (const span of spans) {
    const signature = span.payload.attributes["tx.signature"];
    if (signature) return signature;
  }
  return null;
}

/** Lines for a settle span. Missing fields are omitted. */
export function settleLines(event: StoredEvent): string[] | null {
  const name = spanName(event);
  if (name !== "exchange.settle" && name !== "agent.settle") return null;
  const attributes = event.payload.attributes;
  const lines: string[] = [];
  const fill = attributes["fill.id"];
  if (fill) lines.push(`fill id  ${fill}`);
  const seen = new Set<string>();
  for (const link of event.payload.causalLinks) {
    const key = `${link.attribute} ${link.target}`;
    if (seen.has(key)) continue;
    seen.add(key);
    lines.push(`link ${link.attribute}  ${link.target}`);
  }
  for (const [key, value] of Object.entries(attributes)) {
    if (!key.startsWith("link.")) continue;
    const label = `${key} ${value}`;
    if (seen.has(label)) continue;
    seen.add(label);
    lines.push(`link ${key}  ${value}`);
  }
  const slot = attributes["solana.slot"] ?? attributes["slot"] ?? attributes["tx.slot"];
  if (slot) lines.push(`solana slot  ${slot}`);
  const signature = attributes["tx.signature"];
  if (signature) lines.push(`tx  ${signature}`);
  const ms = spanMillis(event);
  if (ms != null) lines.push(`settle ms  ${ms}`);
  return lines.length === 0 ? null : lines;
}

export function initials(name: string): string {
  const parts = name.split(/[^A-Za-z0-9]+/).filter((part) => part.length > 0);
  if (parts.length >= 2) {
    return `${parts[0]?.[0] ?? ""}${parts[1]?.[0] ?? ""}`.toUpperCase();
  }
  return name.slice(0, 2).toUpperCase();
}

export function clock(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toISOString().slice(11, 23);
}

function spanMillis(event: StoredEvent): number | null {
  const opened = Date.parse(event.payload.startedAt ?? "");
  const closed = Date.parse(event.payload.closedAt ?? "");
  if (Number.isNaN(opened) || Number.isNaN(closed) || closed < opened) return null;
  return Math.round(closed - opened);
}

function factual(span: string, attributes: Record<string, string>, source: string): string {
  const agent = attributes["agent.id"];
  const who = agent ?? source;
  const bill = attributes["bill.id"];
  const fill = attributes["fill.id"];
  const order = attributes["order.id"];
  switch (span) {
    case "agent.decide": {
      const kind = attributes["decision.kind"];
      return kind ? `${who} decided to ${kind}.` : `${who} recorded a decision.`;
    }
    case "agent.approve":
      return bill ? `${who} approved ${bill}.` : `${who} recorded an approval.`;
    case "agent.pay_bill":
    case "ledger.agent.pay_bill":
      return bill ? `${who} recorded a payment for ${bill}.` : `${who} recorded a payment.`;
    case "agent.settle":
      return fill ? `${who} recorded settlement of fill ${fill}.` : `${who} recorded a settlement.`;
    case "exchange.submit_order":
      return order ? `The exchange accepted order ${order}.` : "The exchange accepted an order.";
    case "exchange.match":
      return order ? `The exchange matched order ${order}.` : "The exchange matched an order.";
    case "exchange.fill":
      return fill ? `The exchange filled ${fill}.` : "The exchange recorded a fill.";
    case "exchange.settle":
      return fill ? `The exchange settled fill ${fill}.` : "The exchange settled a fill.";
    case "ledger.exchange.payment":
      return "The exchange recorded an extra payment.";
    case "ledger.exchange.settled":
      return "The exchange recorded a settled obligation.";
    case "ledger.exchange.credit":
      return "The exchange recorded a credit.";
    default:
      return `${source} recorded ${span}.`;
  }
}

function capitalize(text: string): string {
  const trimmed = text.trim().replace(/\.$/, "");
  if (trimmed.length === 0) return "Evidence is missing.";
  return `${trimmed.charAt(0).toUpperCase()}${trimmed.slice(1)}.`;
}

function rootOf(id: string, parent: Map<string, string>): string {
  const seen = new Set<string>();
  let current = id;
  while (!seen.has(current)) {
    const next = parent.get(current);
    if (!next) return current;
    seen.add(current);
    current = next;
  }
  return id;
}

function dedupe(events: StoredEvent[]): StoredEvent[] {
  const seen = new Set<string>();
  const unique: StoredEvent[] = [];
  for (const event of events) {
    const key = `${event.id}|${event.observedAt}`;
    if (seen.has(key)) continue;
    seen.add(key);
    unique.push(event);
  }
  return unique;
}

function byTime(a: StoredEvent, b: StoredEvent): number {
  if (a.observedAt < b.observedAt) return -1;
  if (a.observedAt > b.observedAt) return 1;
  if (a.id < b.id) return -1;
  if (a.id > b.id) return 1;
  return 0;
}

function parsePayload(value: unknown): StoredPayload {
  if (!isRecord(value)) {
    return { attributes: {}, causalLinks: [] };
  }
  const sentinel = parseSentinel(value.sentinel);
  return {
    span: optionalString(value.span),
    spanId: optionalString(value.span_id),
    parentSpanId: optionalString(value.parent_span_id),
    startedAt: optionalString(value.started_at),
    closedAt: optionalString(value.closed_at),
    attributes: parseAttributes(value.attributes),
    causalLinks: parseLinks(value.causal_links),
    adapted: typeof value.adapted === "boolean" ? value.adapted : undefined,
    evidence: optionalString(value.evidence),
    reason: optionalString(value.reason),
    sentinel,
  };
}

function parseAttributes(value: unknown): Record<string, string> {
  if (!isRecord(value)) return {};
  const attributes: Record<string, string> = {};
  for (const [key, item] of Object.entries(value)) {
    if (typeof item === "string") attributes[key] = item;
    else if (typeof item === "number" || typeof item === "boolean") attributes[key] = String(item);
  }
  return attributes;
}

function parseLinks(value: unknown): CausalLink[] {
  if (!Array.isArray(value)) return [];
  const links: CausalLink[] = [];
  for (const item of value) {
    if (!isRecord(item)) continue;
    if (typeof item.attribute !== "string" || typeof item.target !== "string") continue;
    links.push({
      attribute: item.attribute,
      target: item.target,
      resolved: item.resolved === true,
    });
  }
  return links;
}

function parseSentinel(value: unknown): SentinelSummary | undefined {
  if (!isRecord(value)) return undefined;
  const predecessors = Array.isArray(value.predecessors)
    ? value.predecessors.filter((item): item is string => typeof item === "string")
    : [];
  return {
    eventId: optionalString(value.event_id),
    kind: optionalString(value.kind),
    source: optionalString(value.source),
    tier: optionalString(value.tier),
    dataClass: optionalString(value.data_class),
    predecessors,
  };
}

function optionalString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
