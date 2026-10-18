import { describe, expect, test } from "bun:test";
import { applyStreamEvent, attentionCount, parseStreamData } from "./reduce";
import { emptySnapshot, TEN_MINUTES_MS, type MetricsSnapshot, type VerdictItem } from "./types";

function verdict(partial: Pick<VerdictItem, "id" | "outcome" | "title"> & Partial<VerdictItem>): VerdictItem {
  return {
    evidence: "",
    specId: "",
    reasonCode: null,
    amountCents: null,
    assignedToMe: false,
    jevUnknown: false,
    recordedAt: "",
    ...partial,
  };
}

describe("applyStreamEvent", () => {
  test("starts empty", () => {
    const snapshot = emptySnapshot();
    expect(snapshot.eventsIngested).toBe(0);
    expect(snapshot.liveMonitors).toBe(0);
    expect(snapshot.verdicts).toEqual([]);
    expect(attentionCount(snapshot.verdicts)).toBe(0);
  });

  test("places each outcome in its own column and never treats a gap as satisfy", () => {
    let snapshot = emptySnapshot();
    snapshot = applyStreamEvent(snapshot, {
      kind: "verdict",
      verdict: verdict({ id: "VR-1", outcome: "Violate", title: "Paid twice" }),
    });
    snapshot = applyStreamEvent(snapshot, {
      kind: "verdict",
      verdict: verdict({ id: "VR-2", outcome: "Inconclusive", title: "Approval not seen" }),
    });
    snapshot = applyStreamEvent(snapshot, {
      kind: "verdict",
      verdict: verdict({ id: "VR-3", outcome: "Satisfy", title: "Order filled" }),
    });

    expect(snapshot.verdicts.map((item) => item.outcome)).toEqual([
      "Satisfy",
      "Inconclusive",
      "Violate",
    ]);
    expect(attentionCount(snapshot.verdicts)).toBe(2);
    expect(snapshot.connection).toBe("live");
  });

  test("replaces a verdict with the same id", () => {
    let snapshot = emptySnapshot();
    snapshot = applyStreamEvent(snapshot, {
      kind: "verdict",
      verdict: verdict({ id: "VR-1", outcome: "Inconclusive", title: "Waiting" }),
    });
    snapshot = applyStreamEvent(snapshot, {
      kind: "verdict",
      verdict: verdict({ id: "VR-1", outcome: "Violate", title: "Paid twice" }),
    });
    expect(snapshot.verdicts).toHaveLength(1);
    expect(snapshot.verdicts[0]?.outcome).toBe("Violate");
  });

  test("ignores an outcome outside the three stored values", () => {
    const parsed = parseStreamData(
      JSON.stringify({ kind: "verdict", verdict: { id: "VR-9", outcome: "Fraud", title: "Nope" } }),
    );
    expect(parsed).toBeNull();
    expect(emptySnapshot().verdicts).toEqual([]);
  });

  test("patches counters without inventing verdicts", () => {
    const snapshot = applyStreamEvent(emptySnapshot(), {
      kind: "counters",
      counters: { eventsIngested: 12, liveMonitors: 3, settledOnSolana: 1, jevUnknown: 2 },
    });
    expect(snapshot.eventsIngested).toBe(12);
    expect(snapshot.liveMonitors).toBe(3);
    expect(snapshot.settledOnSolana).toBe(1);
    expect(snapshot.jevUnknown).toBe(2);
    expect(snapshot.verdicts).toEqual([]);
  });

  test("keeps rate samples inside ten minutes and updates the event rate", () => {
    const latest = 1_700_000_000_000;
    let snapshot = applyStreamEvent(emptySnapshot(), {
      kind: "rates",
      point: { t: latest - TEN_MINUTES_MS - 1, agentLog: 9, exchange: 0, solana: 0 },
    });
    snapshot = applyStreamEvent(snapshot, {
      kind: "rates",
      point: { t: latest, agentLog: 4, exchange: 2, solana: 0 },
    });
    expect(snapshot.rates).toEqual([{ t: latest, agentLog: 4, exchange: 2, solana: 0 }]);
    expect(snapshot.eventsPerSecond).toBe(6);
    expect(snapshot.sourceCount).toBe(2);
  });

  test("monitor samples set the live count from the with-GC line", () => {
    const snapshot = applyStreamEvent(emptySnapshot(), {
      kind: "monitors",
      point: { t: 10, withGc: 42, withoutGc: 1284 },
    });
    expect(snapshot.liveMonitors).toBe(42);
    expect(snapshot.monitors[0]?.withoutGc).toBe(1284);
  });

  test("hello replaces the snapshot", () => {
    const next: MetricsSnapshot = {
      ...emptySnapshot(),
      eventsIngested: 4,
      verdicts: [verdict({ id: "VR-1", outcome: "Satisfy", title: "Held" })],
    };
    const snapshot = applyStreamEvent(emptySnapshot(), { kind: "hello", snapshot: next });
    expect(snapshot.eventsIngested).toBe(4);
    expect(snapshot.verdicts).toHaveLength(1);
    expect(snapshot.connection).toBe("live");
  });
});

describe("parseStreamData", () => {
  test("rejects junk and unknown kinds", () => {
    expect(parseStreamData("not-json")).toBeNull();
    expect(parseStreamData(JSON.stringify({ kind: "sentry" }))).toBeNull();
  });

  test("accepts a verdict and a rate point", () => {
    const verdictEvent = parseStreamData(
      JSON.stringify({
        kind: "verdict",
        verdict: { id: "VR-1", outcome: "Inconclusive", title: "Source missing" },
      }),
    );
    expect(verdictEvent?.kind).toBe("verdict");
    const rates = parseStreamData(
      JSON.stringify({ kind: "rates", point: { t: 1, agentLog: 1, exchange: 0, solana: 0 } }),
    );
    expect(rates?.kind).toBe("rates");
  });

  test("keeps timeline evidence and does not invent a binding comparison", () => {
    const event = parseStreamData(
      JSON.stringify({
        kind: "verdict",
        verdict: {
          id: "11111111-1111-4111-8111-111111111111",
          outcome: "Violate",
          title: "needs review",
          specId: "ledger.bill.no-second-payment",
          binding: "bill-9",
          timeline: [
            { eventId: "e1", observedAt: "2026-10-18T00:00:00Z", source: "agent.payer", kind: "pay", role: "payment" },
          ],
        },
      }),
    );
    expect(event?.kind).toBe("verdict");
    if (event?.kind !== "verdict") return;
    const snapshot = applyStreamEvent(emptySnapshot(), event);
    expect(snapshot.verdicts[0]?.timeline?.[0]?.source).toBe("agent.payer");
    expect(snapshot.verdicts[0]?.binding).toBe("bill-9");
    expect(snapshot.verdicts[0]?.title).toBe("needs review");
    expect(snapshot.verdicts[0]?.comparison).toBeNull();
    expect(JSON.stringify(snapshot.verdicts[0])).not.toContain("fraud");
  });

  test("keeps a timeline step that has a role and no source", () => {
    const event = parseStreamData(
      JSON.stringify({
        kind: "verdict",
        verdict: {
          id: "v-role",
          outcome: "Violate",
          title: "needs review",
          timeline: [{ eventId: "pay-2", role: "second-payment", kind: "ledger.exchange.payment" }],
        },
      }),
    );
    expect(event?.kind).toBe("verdict");
    if (event?.kind !== "verdict") return;
    const snapshot = applyStreamEvent(emptySnapshot(), event);
    expect(snapshot.verdicts[0]?.timeline).toEqual([
      {
        eventId: "pay-2",
        observedAt: "",
        source: "",
        kind: "ledger.exchange.payment",
        role: "second-payment",
      },
    ]);
  });

  test("keeps Same, Different, and Unknown and drops any other comparison", () => {
    const kept = parseStreamData(
      JSON.stringify({
        kind: "verdict",
        verdict: {
          id: "v-bind",
          outcome: "Inconclusive",
          title: "inconclusive",
          comparison: {
            exact: "Different",
            fuzzy: { outcome: "Same", reasonCode: "normalized-name" },
            jev: "Unknown",
          },
        },
      }),
    );
    expect(kept?.kind).toBe("verdict");
    if (kept?.kind !== "verdict") return;
    const snapshot = applyStreamEvent(emptySnapshot(), kept);
    expect(snapshot.verdicts[0]?.comparison).toEqual({
      exact: { outcome: "Different", reasonCode: null },
      fuzzy: { outcome: "Same", reasonCode: "normalized-name" },
      jev: { outcome: "Unknown", reasonCode: null },
    });
    expect(snapshot.verdicts[0]?.jevUnknown).toBe(true);

    const dropped = parseStreamData(
      JSON.stringify({
        kind: "verdict",
        verdict: {
          id: "v-drop",
          outcome: "Satisfy",
          title: "satisfied",
          comparison: { exact: "Fraud", fuzzy: "Fraud", jev: "Fraud" },
        },
      }),
    );
    expect(dropped?.kind).toBe("verdict");
    if (dropped?.kind !== "verdict") return;
    expect(applyStreamEvent(emptySnapshot(), dropped).verdicts[0]?.comparison).toBeNull();
  });

  test("keeps raw rank scores and treats a bad score as unknown", () => {
    const event = parseStreamData(
      JSON.stringify({
        kind: "verdict",
        verdict: {
          id: "v-rank",
          outcome: "Violate",
          title: "needs review",
          rank: { money: 3.2, evidence: null, ambiguity: 1, intent: 9, input: { model: "jev-latest" } },
        },
      }),
    );
    expect(event?.kind).toBe("verdict");
    if (event?.kind !== "verdict") return;
    const verdict = applyStreamEvent(emptySnapshot(), event).verdicts[0];
    expect(verdict?.rank).toEqual({
      money: 3.2,
      evidence: null,
      ambiguity: 1,
      intent: null,
      input: { model: "jev-latest" },
      output: undefined,
    });
    expect(JSON.stringify(verdict)).not.toContain("fraud");
  });
});
