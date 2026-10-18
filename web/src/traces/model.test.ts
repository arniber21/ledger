import { describe, expect, test } from "bun:test";
import {
  applyTracesEvent,
  evidenceSentence,
  groupTraces,
  parseStoredEvent,
  parseTracesData,
  settleLines,
  traceFor,
} from "./model";
import { emptyTraces } from "./model";

const parent = {
  id: "11111111-1111-4111-8111-111111111111",
  observed_at: "2026-10-18T12:00:00Z",
  source: "agent.payer",
  kind: "agent.decide",
  data_class: "tenant",
  payload: {
    span: "agent.decide",
    span_id: "1",
    started_at: "2026-10-18T12:00:00.000Z",
    closed_at: "2026-10-18T12:00:00.400Z",
    attributes: { "agent.id": "payer", "decision.kind": "pay" },
    causal_links: [],
    adapted: false,
  },
};

const child = {
  id: "22222222-2222-4222-8222-222222222222",
  observed_at: "2026-10-18T12:00:01Z",
  source: "agent.payer",
  kind: "ledger.agent.pay_bill",
  data_class: "tenant",
  payload: {
    span: "agent.pay_bill",
    span_id: "2",
    parent_span_id: "1",
    started_at: "2026-10-18T12:00:00.500Z",
    closed_at: "2026-10-18T12:00:01.000Z",
    attributes: {
      "agent.id": "payer",
      "bill.id": "bill-1",
      "fault.name": "double-pay",
    },
    causal_links: [{ attribute: "link.approval_id", target: "apr-1", resolved: false }],
    adapted: true,
    evidence: "inconclusive",
    reason: "unresolved causal link",
    sentinel: {
      event_id: "22222222-2222-4222-8222-222222222222",
      kind: "ledger.agent.pay_bill",
      source: "agent.payer",
      tier: "contributing",
      data_class: "tenant",
      predecessors: [],
    },
  },
};

const settle = {
  id: "33333333-3333-4333-8333-333333333333",
  observed_at: "2026-10-18T12:00:02Z",
  source: "exchange",
  kind: "exchange.settle",
  data_class: "tenant",
  payload: {
    span: "exchange.settle",
    span_id: "5",
    parent_span_id: "1",
    started_at: "2026-10-18T12:00:01.000Z",
    closed_at: "2026-10-18T12:00:01.840Z",
    attributes: {
      "fill.id": "1",
      "tx.signature": "5".repeat(64),
      "link.fill_id": "1",
    },
    causal_links: [{ attribute: "link.fill_id", target: "1", resolved: true }],
    adapted: false,
  },
};

describe("parseStoredEvent", () => {
  test("keeps hypertable columns and ingest payload keys", () => {
    const event = parseStoredEvent(child);
    expect(event?.source).toBe("agent.payer");
    expect(event?.kind).toBe("ledger.agent.pay_bill");
    expect(event?.dataClass).toBe("tenant");
    expect(event?.payload.span).toBe("agent.pay_bill");
    expect(event?.payload.parentSpanId).toBe("1");
    expect(event?.payload.sentinel?.tier).toBe("contributing");
    expect(event?.payload.evidence).toBe("inconclusive");
  });

  test("drops a row that is missing a column", () => {
    expect(parseStoredEvent({ id: "x", payload: {} })).toBeNull();
  });
});

describe("groupTraces", () => {
  test("joins a child to its parent span and leaves an orphan alone", () => {
    const decide = parseStoredEvent(parent);
    const pay = parseStoredEvent(child);
    const settled = parseStoredEvent(settle);
    if (!decide || !pay || !settled) throw new Error("fixture");
    const traces = groupTraces([decide, pay, settled]);
    expect(traces).toHaveLength(1);
    expect(traces[0]?.id).toBe(decide.id);
    expect(traces[0]?.spans.map((span) => span.id)).toEqual([decide.id, pay.id, settled.id]);
    expect(traceFor(traces, pay.id)?.id).toBe(decide.id);
  });

  test("starts empty and appends a live span without duplicating the hello", () => {
    const decide = parseStoredEvent(parent);
    if (!decide) throw new Error("fixture");
    const hello = parseTracesData(JSON.stringify({ kind: "hello", events: [parent] }));
    const again = parseTracesData(JSON.stringify({ kind: "span", event: parent }));
    const pay = parseTracesData(JSON.stringify({ kind: "span", event: child }));
    if (!hello || !again || !pay) throw new Error("message");
    let snapshot = applyTracesEvent(emptyTraces(), hello);
    snapshot = applyTracesEvent(snapshot, again);
    snapshot = applyTracesEvent(snapshot, pay);
    expect(snapshot.connection).toBe("live");
    expect(snapshot.events.map((event) => event.id)).toEqual([decide.id, child.id]);
  });
});

describe("evidenceSentence", () => {
  test("says needs review for a double-pay fault and never fraud", () => {
    const event = parseStoredEvent(child);
    if (!event) throw new Error("fixture");
    const sentence = evidenceSentence(event);
    expect(sentence.startsWith("Needs review.")).toBe(true);
    expect(sentence.toLowerCase().includes("fraud")).toBe(false);
  });

  test("keeps a dangling link inconclusive when no fault is present", () => {
    const event = parseStoredEvent({
      ...child,
      payload: {
        ...child.payload,
        attributes: { "agent.id": "payer", "bill.id": "bill-1" },
      },
    });
    if (!event) throw new Error("fixture");
    expect(evidenceSentence(event)).toBe("Inconclusive. Unresolved causal link.");
  });
});

describe("settleLines", () => {
  test("shows fill, link, tx, and settle ms, and skips a missing slot", () => {
    const event = parseStoredEvent(settle);
    if (!event) throw new Error("fixture");
    const lines = settleLines(event);
    expect(lines?.[0]).toBe("fill id  1");
    expect(lines?.some((line) => line.startsWith("link "))).toBe(true);
    expect(lines?.some((line) => line.startsWith("tx  "))).toBe(true);
    expect(lines?.some((line) => line.startsWith("settle ms  "))).toBe(true);
    expect(lines?.some((line) => line.includes("slot"))).toBe(false);
  });
});
