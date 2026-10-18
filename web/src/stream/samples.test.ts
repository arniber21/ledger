import { describe, expect, test } from "bun:test";
import { applySamples, applyStreamEvent } from "./reduce";
import { emptySnapshot } from "./types";

const body = {
  counters: { eventsIngested: 822, eventsPerSecond: 0.5, sourceCount: 2, liveMonitors: 3, jevJudgments: 7 },
  rates: [{ t: 1, agentLog: 0.1, exchange: 0.2, solana: 0 }, { t: "bad" }],
  attention: [{ t: 1, violate: 2, inconclusive: 1 }],
  monitors: [{ t: 1, withGc: 3, withoutGc: 5 }, { t: 2, withGc: 3 }],
};

describe("applySamples", () => {
  test("fills counters and series, dropping unusable points", () => {
    const next = applySamples(emptySnapshot(), body);
    expect(next.eventsIngested).toBe(822);
    expect(next.liveMonitors).toBe(3);
    expect(next.rates).toHaveLength(1);
    expect(next.attention).toEqual([{ t: 1, violate: 2, inconclusive: 1 }]);
    expect(next.monitors).toEqual([{ t: 1, withGc: 3, withoutGc: 5 }]);
  });

  test("a body that does not parse leaves the snapshot alone", () => {
    const start = emptySnapshot();
    expect(applySamples(start, null)).toBe(start);
    expect(applySamples(start, { rates: [] })).toBe(start);
  });

  test("a recorded scene on the stream wins over polled samples", () => {
    const scene = applyStreamEvent(emptySnapshot(), { kind: "counters", counters: { eventsIngested: 40 } });
    expect(scene.sceneMetrics).toBe(true);
    const next = applySamples(scene, body);
    expect(next.eventsIngested).toBe(40);
  });
});
