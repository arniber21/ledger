import { describe, expect, test } from "bun:test";
import type { VerdictItem } from "../stream/types";
import {
  ageLabel,
  bindingLabel,
  cardSubtitle,
  cardTitle,
  DEFAULT_RANK_WEIGHTS,
  jevChip,
  sourceLabel,
} from "./model";

function verdict(partial: Partial<VerdictItem>): VerdictItem {
  return {
    id: "v-1",
    outcome: "Violate",
    title: "needs review",
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

describe("inbox card fields", () => {
  test("titles follow the outcome and subtitles say why, in plain words", () => {
    const approval = "ledger.bill.approval-before-payment";
    expect(cardTitle(verdict({ outcome: "Violate", specId: approval }))).toBe("Needs Review");
    expect(cardSubtitle(verdict({ outcome: "Violate", specId: approval }))).toBe("Payment without approval");
    expect(cardTitle(verdict({ outcome: "Satisfy", specId: approval }))).toBe("Satisfied");
    expect(cardSubtitle(verdict({ outcome: "Satisfy", specId: approval }))).toBe("Approved before payment");
    expect(cardSubtitle(verdict({ outcome: "Satisfy", specId: "ledger.bill.no-second-payment" }))).toBe("No second payment");
    expect(cardSubtitle(verdict({ outcome: "Satisfy", specId: "ledger.order.fill-or-cancel" }))).toBe("Order filled or cancelled");
    expect(cardTitle(verdict({ outcome: "Inconclusive" }))).toBe("Inconclusive");
    expect(cardSubtitle(verdict({ outcome: "Inconclusive", specId: approval, reasonCode: "prefix-uncertainty" }))).toBe("Approval not seen yet");
    expect(
      cardSubtitle(verdict({ outcome: "Inconclusive", specId: "ledger.bill.no-second-payment", reasonCode: "missing-evidence" })),
    ).toBe("Evidence missing");
    expect(cardSubtitle(verdict({ outcome: "Violate", specId: "ledger.authored.cap-spend" }))).toBe("Cap spend");
    expect(cardSubtitle(verdict({ outcome: "Violate", specId: "" }))).toBe("Verdict");
  });

  test("binding label shortens ids and drops generic names", () => {
    expect(bindingLabel(verdict({ binding: "bill" }))).toBeNull();
    expect(bindingLabel(verdict({ binding: "bill-1" }))).toBe("bill-1");
    expect(bindingLabel(verdict({ binding: "order-3115343b-1720-453d-ae60-d72e895df06e" }))).toBe("order-3115343b");
  });

  test("source reads the evidence trail, then falls back to where the monitor listens", () => {
    const step = (source: string) => ({ eventId: "e", observedAt: "", source, kind: "k", role: "r" });
    expect(sourceLabel(verdict({ timeline: [step("agent.payer-907b2c61-edfc-4415-8652-8d6e597d904f")] }))).toBe("Agent payer");
    expect(sourceLabel(verdict({ timeline: [step("exchange")] }))).toBe("Exchange");
    expect(sourceLabel(verdict({ specId: "ledger.order.fill-or-cancel" }))).toBe("Exchange orders");
    expect(sourceLabel(verdict({ specId: "mystery" }))).toBe("No evidence steps");
  });

  test("an implausible stored time has no age", () => {
    expect(ageLabel("2026-10-18T11:50:00Z", NOW)).toBe("10m");
    expect(ageLabel("2023-11-14T22:13:20Z", NOW)).toBe("—");
    expect(ageLabel("2245-10-30T15:13:06Z", NOW)).toBe("—");
    expect(ageLabel("", NOW)).toBe("—");
  });

  test("an unscored verdict has no chip and unknown scores are not zero", () => {
    expect(jevChip(verdict({}), DEFAULT_RANK_WEIGHTS)).toBeNull();
    const unknown = verdict({ rank: { money: null, evidence: 1, ambiguity: null, intent: null } });
    expect(jevChip(unknown, DEFAULT_RANK_WEIGHTS)).toBeNull();
    expect(jevChip({ ...unknown, jevUnknown: true }, DEFAULT_RANK_WEIGHTS)).toBe("Jev unknown");
    const scored = verdict({ rank: { money: 4, evidence: 4, ambiguity: 4, intent: 4 } });
    expect(jevChip(scored, DEFAULT_RANK_WEIGHTS)).toBe("Jev 1.00");
  });
});
