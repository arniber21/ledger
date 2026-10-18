import { describe, expect, test } from "bun:test";
import { applyQuick, exchangeLines, type QuickReply } from "./quick";

function reply(partial: Pick<QuickReply, "status" | "action"> & { phrase?: string }): QuickReply {
  return {
    phrase: partial.phrase ?? "violations over $1,000 by agent-2",
    status: partial.status,
    action: partial.action,
    exchange: {
      request: { phrase: partial.phrase ?? "violations over $1,000 by agent-2" },
      response: {
        answers: {
          handler: { type: "choice", choice: "filter_inbox", probabilities: { filter_inbox: 0.92 } },
        },
      },
    },
  };
}

describe("quick actions", () => {
  test("three filter phrases land as chips", () => {
    const violations = applyQuick(
      reply({
        status: "routed",
        action: { kind: "filter_inbox", verdict: "Violate", amount: "over-1000", agent: "agent-2" },
      }),
      new Set(),
    );
    expect(violations).toEqual({
      type: "inbox",
      search: { verdict: "Violate", agent: "agent-2", spec: undefined, amount: "over-1000" },
      notice: "Inbox filters · Needs review · Over $1,000 · agent-2",
    });

    const inconclusive = applyQuick(
      reply({
        phrase: "filter the inbox to inconclusive verdicts on approval before payment",
        status: "routed",
        action: {
          kind: "filter_inbox",
          verdict: "Inconclusive",
          spec: "ledger.bill.approval-before-payment",
        },
      }),
      new Set(),
    );
    expect(inconclusive.type).toBe("inbox");
    if (inconclusive.type === "inbox") {
      expect(inconclusive.search.verdict).toBe("Inconclusive");
      expect(inconclusive.search.spec).toBe("ledger.bill.approval-before-payment");
    }

    const satisfied = applyQuick(
      reply({
        phrase: "filter the inbox to satisfied verdicts by agent-1",
        status: "routed",
        action: { kind: "filter_inbox", verdict: "Satisfy", agent: "agent-1" },
      }),
      new Set(),
    );
    expect(satisfied.type).toBe("inbox");
    if (satisfied.type === "inbox") {
      expect(satisfied.search.verdict).toBe("Satisfy");
      expect(satisfied.search.agent).toBe("agent-1");
    }
  });

  test("an unsupported phrase is text search and not a pass", () => {
    const applied = applyQuick(
      reply({
        phrase: "what is the weather today",
        status: "unsupported",
        action: { kind: "text_search" },
      }),
      new Set(),
    );
    expect(applied).toEqual({
      type: "text",
      search: { q: "what is the weather today" },
      notice: "Not a quick action. Showing the phrase as text search.",
    });
    expect(JSON.stringify(applied)).not.toContain("fraud");
    expect(JSON.stringify(applied)).not.toContain("Satisfy");
  });

  test("unknown is text search", () => {
    const applied = applyQuick(
      reply({ status: "unknown", action: { kind: "text_search" } }),
      new Set(),
    );
    expect(applied.type).toBe("text");
    if (applied.type === "text") {
      expect(applied.search.verdict).toBeUndefined();
      expect(applied.notice).toContain("Unknown");
    }
  });

  test("a missing verdict is not opened and no verdict is invented", () => {
    const applied = applyQuick(
      reply({
        phrase: "open verdict not-on-the-stream",
        status: "routed",
        action: { kind: "open_verdict", id: "not-on-the-stream" },
      }),
      new Set(["v-real"]),
    );
    expect(applied).toEqual({ type: "missing", notice: "That verdict is not on the stream." });
    expect(applied).not.toHaveProperty("id");
    expect(JSON.stringify(applied)).not.toContain("needs review");
  });

  test("a known verdict id opens without a new title", () => {
    const applied = applyQuick(
      reply({
        phrase: "open verdict v-real",
        status: "routed",
        action: { kind: "open_verdict", id: "v-real" },
      }),
      new Set(["v-real"]),
    );
    expect(applied).toEqual({
      type: "open",
      id: "v-real",
      notice: "Opened a verdict already on the stream.",
    });
  });

  test("fault injection asks for confirmation and does not create a verdict", () => {
    const applied = applyQuick(
      reply({
        phrase: "inject a double-pay fault",
        status: "routed",
        action: { kind: "inject_fault", fault: "double-pay" },
      }),
      new Set(),
    );
    expect(applied).toEqual({ type: "confirm", fault: "double-pay" });
    expect(JSON.stringify(applied)).not.toContain("verdict");
    expect(JSON.stringify(applied)).not.toContain("fraud");
  });

  test("exchange lines keep the judgment and drop explanation text", () => {
    const routed = reply({
      status: "routed",
      action: { kind: "filter_inbox", verdict: "Violate" },
    });
    routed.exchange.response = {
      answers: {
        handler: {
          type: "choice",
          choice: "filter_inbox",
          probabilities: { filter_inbox: 0.92 },
          explanation: "do not show this prose",
        },
      },
    };
    const lines = exchangeLines(routed);
    expect(lines[0]).toContain("violations over $1,000 by agent-2");
    expect(lines.some((line) => line.includes("filter_inbox") && line.includes("0.92"))).toBe(true);
    expect(lines.join(" ")).not.toContain("prose");
  });
});
