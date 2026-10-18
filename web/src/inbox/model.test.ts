import { describe, expect, test } from "bun:test";
import { outcomeLabel } from "../stream/reduce";
import type { VerdictItem } from "../stream/types";
import {
  countNeedsReview,
  filterChips,
  groupItems,
  lastReviewedLabel,
  orderedIds,
  parseInboxSearch,
  parseTriagePayload,
  prepareInbox,
  queryFromSearch,
  savedInboxSearch,
  type Triage,
} from "./model";

function verdict(partial: Pick<VerdictItem, "id" | "outcome" | "title"> & Partial<VerdictItem>): VerdictItem {
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
    ...partial,
  };
}

const NOW = Date.parse("2026-10-18T12:00:00Z");

describe("inbox model", () => {
  test("starts empty and does not invent verdicts", () => {
    const prepared = prepareInbox([], [], queryFromSearch({}));
    expect(prepared.items).toEqual([]);
    expect(prepared.groups).toEqual([]);
    expect(prepared.attention).toBe(0);
    expect(countNeedsReview([], [])).toBe(0);
  });

  test("keeps violate and inconclusive and labels violate as needs review", () => {
    const prepared = prepareInbox(
      [
        verdict({ id: "v-1", outcome: "Violate", title: "needs review", specId: "no-second-payment" }),
        verdict({ id: "v-2", outcome: "Inconclusive", title: "approval missing" }),
        verdict({ id: "v-3", outcome: "Satisfy", title: "held" }),
      ],
      [],
      queryFromSearch({}),
    );
    expect(prepared.items.map((item) => item.verdict.id)).toEqual(["v-1", "v-2"]);
    expect(prepared.items.every((item) => item.status === "New")).toBe(true);
    expect(outcomeLabel("Violate")).toBe("Needs review");
    expect(JSON.stringify(prepared)).not.toContain("fraud");
    expect(prepared.attention).toBe(2);
  });

  test("puts each status in its own board column", () => {
    const triage: Triage[] = [
      { verdictId: "v-2", status: "Reviewing", assignee: "me", updatedAt: "", updatedBy: "me" },
      { verdictId: "v-3", status: "Confirmed", assignee: null, updatedAt: "", updatedBy: "me" },
      { verdictId: "v-4", status: "Dismissed", assignee: null, updatedAt: "", updatedBy: "me" },
    ];
    const prepared = prepareInbox(
      [
        verdict({ id: "v-1", outcome: "Violate", title: "needs review" }),
        verdict({ id: "v-2", outcome: "Inconclusive", title: "missing" }),
        verdict({ id: "v-3", outcome: "Violate", title: "needs review" }),
        verdict({ id: "v-4", outcome: "Violate", title: "needs review" }),
      ],
      triage,
      queryFromSearch({}),
    );
    const ids = orderedIds("board", prepared.items, prepared.groups, new Set());
    expect(ids).toEqual(["v-1", "v-2", "v-3", "v-4"]);
    expect(prepared.attention).toBe(2);
  });

  test("groups the list by status, verdict, agent, or spec", () => {
    const items = prepareInbox(
      [
        verdict({
          id: "v-1",
          outcome: "Violate",
          title: "needs review",
          specId: "spec-a",
          timeline: [{ eventId: "e1", observedAt: "", source: "agent.payer", kind: "pay", role: "payment" }],
        }),
        verdict({ id: "v-2", outcome: "Inconclusive", title: "missing", specId: "spec-b" }),
      ],
      [],
      queryFromSearch({ group: "agent" }),
    ).items;
    expect(groupItems(items, "agent").map((group) => group.label)).toEqual(["agent.payer", "No source"]);
    expect(groupItems(items, "spec").map((group) => group.key)).toEqual(["spec-a", "spec-b"]);
    expect(groupItems(items, "verdict").map((group) => group.label)).toEqual(["Needs review", "Inconclusive"]);
    const collapsed = orderedIds("list", items, groupItems(items, "status"), new Set(["New"]));
    expect(collapsed).toEqual([]);
  });

  test("filters amount, assignee, and jev without marking a verdict unknown", () => {
    const verdicts = [
      verdict({ id: "big", outcome: "Violate", title: "needs review", amountCents: 100_000, assignedToMe: true }),
      verdict({ id: "small", outcome: "Violate", title: "needs review", amountCents: 99_999 }),
      verdict({ id: "unknown", outcome: "Inconclusive", title: "jev", jevUnknown: true }),
      verdict({ id: "plain", outcome: "Inconclusive", title: "gap" }),
    ];
    expect(prepareInbox(verdicts, [], queryFromSearch(savedInboxSearch("over-1000"))).items.map((item) => item.verdict.id)).toEqual([
      "big",
    ]);
    expect(prepareInbox(verdicts, [], queryFromSearch(savedInboxSearch("assigned"))).items.map((item) => item.verdict.id)).toEqual([
      "big",
    ]);
    const unknowns = prepareInbox(verdicts, [], queryFromSearch(savedInboxSearch("unknowns"))).items;
    expect(unknowns.map((item) => item.verdict.id)).toEqual(["unknown"]);
    expect(unknowns.every((item) => item.verdict.jevUnknown)).toBe(true);
  });

  test("sorts amount with missing amounts last and reads the agent only from the timeline", () => {
    const prepared = prepareInbox(
      [
        verdict({ id: "none", outcome: "Violate", title: "needs review", amountCents: null }),
        verdict({ id: "low", outcome: "Violate", title: "needs review", amountCents: 10 }),
        verdict({ id: "high", outcome: "Violate", title: "needs review", amountCents: 50 }),
      ],
      [],
      queryFromSearch({ sort: "amount" }),
    );
    expect(prepared.items.map((item) => item.verdict.id)).toEqual(["high", "low", "none"]);
    expect(prepared.items.every((item) => item.agent === "")).toBe(true);
  });

  test("dismissed view is separate from the calm empty inbox", () => {
    const triage: Triage[] = [
      { verdictId: "v-1", status: "Dismissed", assignee: null, updatedAt: "2026-10-18T11:00:00Z", updatedBy: "me" },
    ];
    const verdicts = [verdict({ id: "v-1", outcome: "Violate", title: "needs review" })];
    expect(prepareInbox(verdicts, triage, queryFromSearch({})).attention).toBe(0);
    expect(prepareInbox(verdicts, triage, queryFromSearch({ dismissed: true })).items).toHaveLength(1);
  });

  test("drops unknown statuses and keeps a last-reviewed line", () => {
    expect(parseTriagePayload({ items: [{ verdictId: "v", status: "Fraud" }, { verdictId: "ok", status: "Confirmed", updatedAt: "2026-10-18T10:00:00Z", updatedBy: "me" }] })).toEqual([
      {
        verdictId: "ok",
        status: "Confirmed",
        assignee: null,
        updatedAt: "2026-10-18T10:00:00Z",
        updatedBy: "me",
      },
    ]);
    expect(lastReviewedLabel(parseTriagePayload({ items: [{ verdictId: "ok", status: "Confirmed", updatedAt: "2026-10-18T10:00:00Z", updatedBy: "me" }] }), NOW)).toBe(
      "Last reviewed 2h ago by me",
    );
    expect(lastReviewedLabel([], NOW)).toBe("No verdict has been reviewed.");
  });

  test("priority weights reorder scored verdicts and unknown stays on severity order", () => {
    const money = {
      money: 4,
      evidence: 0,
      ambiguity: 0,
      intent: 0,
    };
    const evidence = {
      money: 0,
      evidence: 4,
      ambiguity: 4,
      intent: 4,
    };
    const verdicts = [
      verdict({ id: "money-high", outcome: "Violate", title: "needs review", rank: money }),
      verdict({ id: "evidence-high", outcome: "Violate", title: "needs review", rank: evidence }),
      verdict({ id: "unknown-violate", outcome: "Violate", title: "needs review", rank: { ...money, money: null } }),
      verdict({ id: "unknown-gap", outcome: "Inconclusive", title: "inconclusive", rank: { money: null, evidence: null, ambiguity: null, intent: null } }),
    ];
    const moneyWeights = { money: 1, evidence: 0, ambiguity: 0, intent: 0 };
    const evidenceWeights = { money: 0, evidence: 1, ambiguity: 0, intent: 0 };
    const byMoney = prepareInbox(verdicts, [], queryFromSearch({ sort: "priority" }), moneyWeights);
    const byEvidence = prepareInbox(verdicts, [], queryFromSearch({ sort: "priority" }), evidenceWeights);
    expect(byMoney.items.map((item) => item.verdict.id)).toEqual([
      "money-high",
      "evidence-high",
      "unknown-violate",
      "unknown-gap",
    ]);
    expect(byEvidence.items.map((item) => item.verdict.id)).toEqual([
      "evidence-high",
      "money-high",
      "unknown-violate",
      "unknown-gap",
    ]);
    for (const prepared of [byMoney, byEvidence]) {
      for (const view of ["board", "list", "split"] as const) {
        expect(orderedIds(view, prepared.items, prepared.groups, new Set())).toEqual(
          prepared.items.map((item) => item.verdict.id),
        );
      }
    }
    expect(JSON.stringify(byMoney)).not.toContain("fraud");
    expect(parseInboxSearch({ sort: "priority" }).sort).toBe("priority");
  });

  test("search params accept the three modes and reject a fake view", () => {
    expect(parseInboxSearch({ view: "split", group: "spec", sort: "id" }).view).toBe("split");
    expect(parseInboxSearch({ view: "fraud" }).view).toBeUndefined();
    expect(parseInboxSearch({ verdict: "pass" }).verdict).toBeUndefined();
    expect(filterChips({ verdict: "Violate", amount: "over-1000" }).map((chip) => chip.label)).toEqual([
      "Verdict · Needs review",
      "Amount · Over $1,000",
    ]);
  });
});
