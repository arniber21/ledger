/**
 * Verdict detail helpers.
 *
 * Timeline steps, signatures, tiers, and binding lines come from the stream
 * or from stored events. Missing fields stay missing.
 */

import type { StoredEvent } from "../traces/model";
import type { TimelineStep, VerdictItem } from "../stream/types";

const ROLE_LABEL: Record<string, string> = {
  payment: "Payment",
  settlement: "Settlement",
  "second-payment": "Second payment",
  credit: "Credit",
  approval: "Approval",
  order: "Order",
  "fill-or-cancel": "Fill or cancel",
  deadline: "Deadline",
  tick: "Tick",
};

/** Reason codes the engine writes, and the timeline role they mark. */
const VIOLATING_ROLE: Record<string, string> = {
  "second-payment": "second-payment",
  "payment-without-approval": "payment",
  "order-unresolved": "deadline",
};

const SETTLE_KINDS = new Set(["exchange.settle", "agent.settle", "ledger.exchange.settled"]);

export function verdictTitle(verdict: Pick<VerdictItem, "title">): string {
  return /fraud/i.test(verdict.title) ? "needs review" : verdict.title;
}

export function entryLabel(step: TimelineStep): string {
  if (step.role && ROLE_LABEL[step.role]) return ROLE_LABEL[step.role];
  if (step.role) return step.role;
  return step.kind || "Event";
}

export function entryDetail(step: TimelineStep): string {
  return [step.kind, step.source, step.eventId].filter((part) => part.length > 0).join(" · ");
}

export function isViolating(verdict: VerdictItem, step: TimelineStep): boolean {
  if (verdict.outcome !== "Violate" || !verdict.reasonCode) return false;
  const role = VIOLATING_ROLE[verdict.reasonCode] ?? verdict.reasonCode;
  return step.role === role;
}

export function agentOf(verdict: VerdictItem): string | null {
  for (const step of verdict.timeline ?? []) {
    if (step.source.startsWith("agent.") && step.source.length > "agent.".length) {
      return step.source.slice("agent.".length);
    }
  }
  return null;
}

export function sourcesOf(verdict: VerdictItem): string | null {
  const sources = new Set((verdict.timeline ?? []).map((step) => step.source).filter((source) => source.length > 0));
  if (sources.size === 0) return null;
  return [...sources].join(", ");
}

export function eventFor(events: readonly StoredEvent[], eventId: string): StoredEvent | undefined {
  return events.find((event) => event.id === eventId);
}

export function tierFor(events: readonly StoredEvent[], eventId: string): string | null {
  return eventFor(events, eventId)?.payload.sentinel?.tier ?? null;
}

export function assuranceOf(events: readonly StoredEvent[], timeline: readonly TimelineStep[]): string | null {
  const tiers = new Set<string>();
  let matched = false;
  for (const step of timeline) {
    const event = eventFor(events, step.eventId);
    if (!event) continue;
    matched = true;
    const tier = event.payload.sentinel?.tier;
    if (tier) tiers.add(tier);
  }
  if (!matched) return null;
  if (tiers.size === 0) return "Not adapted";
  return [...tiers].join(", ");
}

/** Signature text from a stored event. Null when that event is not loaded. */
export function signatureLine(events: readonly StoredEvent[], step: TimelineStep): string | null {
  const event = eventFor(events, step.eventId);
  if (!event) return null;
  const signature = event.payload.attributes["tx.signature"];
  if (signature) return signature;
  const name = event.payload.span ?? event.kind;
  if (SETTLE_KINDS.has(name) || SETTLE_KINDS.has(step.kind)) return "No transaction signature on this event.";
  return null;
}

/** An https Sentry URL stored on a matched event. Anything else is ignored. */
export function sentryUrl(events: readonly StoredEvent[], timeline: readonly TimelineStep[]): string | null {
  for (const step of timeline) {
    const event = eventFor(events, step.eventId);
    if (!event) continue;
    for (const key of ["sentry.url", "sentry.trace_url"]) {
      const value = event.payload.attributes[key];
      if (value && value.startsWith("https://")) return value;
    }
  }
  return null;
}
