import { describe, expect, test } from "bun:test";
import type { StoredEvent } from "../traces/model";
import type { VerdictItem } from "../stream/types";
import { agentOf, counterexampleView, exchangeText, isViolating, scoreLabel, sentryUrl, signatureLine, verdictTitle } from "./model";

function verdict(partial: Partial<VerdictItem> & Pick<VerdictItem, "id" | "outcome" | "title">): VerdictItem {
  return {
    evidence: "",
    specId: "",
    reasonCode: null,
    amountCents: null,
    assignedToMe: false,
    jevUnknown: false,
    recordedAt: "",
    binding: "",
    timeline: [],
    comparison: null,
    ...partial,
  };
}

function step(eventId: string, observedAt: string, role: string) {
  return { eventId, observedAt, source: "exchange", kind: role, role };
}

function event(partial: Partial<StoredEvent> & Pick<StoredEvent, "id">): StoredEvent {
  return {
    observedAt: "",
    source: "exchange",
    kind: "exchange.settle",
    dataClass: "tenant",
    payload: { attributes: {}, causalLinks: [] },
    ...partial,
  };
}

describe("verdict detail", () => {
  test("marks only the timeline role that matches the reason code", () => {
    const item = verdict({
      id: "v",
      outcome: "Violate",
      title: "needs review",
      reasonCode: "second-payment",
      timeline: [
        { eventId: "a", observedAt: "", source: "agent.payer", kind: "pay", role: "payment" },
        { eventId: "b", observedAt: "", source: "exchange", kind: "ledger.exchange.payment", role: "second-payment" },
      ],
    });
    expect(isViolating(item, item.timeline?.[0]!)).toBe(false);
    expect(isViolating(item, item.timeline?.[1]!)).toBe(true);
    expect(isViolating({ ...item, outcome: "Satisfy" }, item.timeline?.[1]!)).toBe(false);
  });

  test("reads the agent from the timeline and does not invent one", () => {
    expect(agentOf(verdict({ id: "v", outcome: "Satisfy", title: "satisfied" }))).toBeNull();
    expect(
      agentOf(
        verdict({
          id: "v",
          outcome: "Violate",
          title: "needs review",
          timeline: [{ eventId: "a", observedAt: "", source: "agent.payer", kind: "pay", role: "payment" }],
        }),
      ),
    ).toBe("payer");
  });

  test("shows a signature only when the stored event has one", () => {
    const step = { eventId: "e", observedAt: "", source: "exchange", kind: "exchange.settle", role: "settlement" };
    expect(signatureLine([], step)).toBeNull();
    expect(signatureLine([event({ id: "e" })], step)).toBe("No transaction signature on this event.");
    expect(
      signatureLine(
        [event({ id: "e", payload: { attributes: { "tx.signature": "sig-1" }, causalLinks: [] } })],
        step,
      ),
    ).toBe("sig-1");
  });

  test("accepts only an https Sentry URL", () => {
    const step = { eventId: "e", observedAt: "", source: "exchange", kind: "pay", role: "payment" };
    expect(sentryUrl([event({ id: "e", payload: { attributes: { "sentry.url": "http://sentry.example/1" }, causalLinks: [] } })], [step])).toBeNull();
    expect(
      sentryUrl(
        [event({ id: "e", payload: { attributes: { "sentry.url": "https://sentry.example/trace/1" }, causalLinks: [] } })],
        [step],
      ),
    ).toBe("https://sentry.example/trace/1");
  });

  test("shows the three double-pay events and hides the rest until expand", () => {
    const item = verdict({
      id: "double-pay",
      outcome: "Violate",
      title: "needs review",
      rank: "ranked",
      timeline: [
        step("pay", "2026-10-18T00:00:01Z", "payment"),
        step("again", "2026-10-18T00:00:05Z", "second-payment"),
        step("again:close", "2026-10-18T00:00:06Z", "credit"),
      ],
      rest: [
        step("tick-1", "2026-10-18T00:00:02Z", "tick"),
        step("settled", "2026-10-18T00:00:03Z", "settlement"),
        step("order-9", "2026-10-18T00:00:04Z", "order"),
      ],
    });
    const closed = counterexampleView(item, false);
    expect(closed.steps.map((entry) => entry.eventId)).toEqual(["pay", "again", "again:close"]);
    expect(closed.hiddenCount).toBe(3);
    expect(closed.unknown).toBe(false);
    const open = counterexampleView(item, true);
    expect(open.steps.map((entry) => entry.eventId)).toEqual([
      "pay",
      "tick-1",
      "settled",
      "order-9",
      "again",
      "again:close",
    ]);
    expect(open.hiddenCount).toBe(3);
  });

  test("a failed ranking shows the full trace in time order", () => {
    const item = verdict({
      id: "double-pay",
      outcome: "Violate",
      title: "needs review",
      rank: "unknown",
      timeline: [
        step("pay", "2026-10-18T00:00:01Z", "payment"),
        step("tick-1", "2026-10-18T00:00:02Z", "tick"),
        step("again", "2026-10-18T00:00:04Z", "second-payment"),
      ],
    });
    const view = counterexampleView(item, false);
    expect(view.unknown).toBe(true);
    expect(view.hiddenCount).toBe(0);
    expect(view.steps.map((entry) => entry.eventId)).toEqual(["pay", "tick-1", "again"]);
    expect(scoreLabel("Unknown")).toBe("Unknown");
    expect(scoreLabel("NotRelevant")).toBe("Not relevant");
    expect(exchangeText('{"explanation":"no"}')).toBe("");
  });

  test("does not display the word fraud as a title", () => {
    expect(verdictTitle(verdict({ id: "v", outcome: "Violate", title: "Fraud" }))).toBe("needs review");
    expect(verdictTitle(verdict({ id: "v", outcome: "Violate", title: "needs review" }))).toBe("needs review");
  });
});
