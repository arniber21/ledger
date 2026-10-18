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
});
