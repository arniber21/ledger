import { describe, expect, test } from "bun:test";
import {
  applyAuthorEvents,
  beginDraft,
  checkDetail,
  composingRun,
  drainSse,
  eventsFromSse,
  failDraft,
  finishDraft,
  policySegments,
  scopeOf,
} from "./reduce";
import type { SpecRun } from "./types";

const policy = "After an agent pays a bill, no second payment may occur on that bill until a credit closes it.";

function frame(event: string, data: unknown): string {
  return `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`;
}

function run(text: string): SpecRun {
  return applyAuthorEvents(beginDraft(composingRun("draft-1"), policy), eventsFromSse(text));
}

describe("authoring stream", () => {
  test("keeps a partial frame until it closes", () => {
    const partial = `${frame("sentence", { text: policy })}event: draft\ndata: {"pattern_id":`;
    const drained = drainSse(partial);
    expect(drained.events.map((event) => event.event)).toEqual(["sentence"]);
    expect(drained.rest.startsWith("event: draft")).toBe(true);
  });

  test("drops a frame that is not the authoring protocol", () => {
    const text = `${frame("sentence", { text: policy })}event: note\ndata: {"text":"hello"}\n\n${frame("check", { verdict: "Maybe" })}`;
    const names = eventsFromSse(text).map((event) => event.event);
    expect(names).toEqual(["sentence"]);
  });

  test("arms a catalogue-valid same match without inventing a score", () => {
    const armed = run(
      [
        frame("sentence", { text: policy }),
        frame("draft", {
          pattern_id: "absence.after-until",
          parameters: {
            binding_attribute: "bill.id",
            after: "ledger.agent.pay_bill",
            lookback: 4,
          },
        }),
        frame("check", { verdict: "Same" }),
        frame("accepted", { pattern_id: "absence.after-until", monitor: "running" }),
      ].join(""),
    );

    expect(armed.steps.map((step) => step.kind)).toEqual(["policy", "draft", "catalogue", "check", "armed"]);
    expect(armed.status).toBe("armed");
    expect(armed.specId).toBe("ledger.authored.absence.after-until");
    expect(armed.draftedBy).toBe("Gemini");
    expect(armed.match).toBe("Same");
    expect(scopeOf(armed.parameters)).toBe("bill.id");
    const check = armed.steps.find((step) => step.kind === "check");
    expect(check).toEqual({ kind: "check", verdict: "Same" });
    expect(checkDetail("Same")).toBe("Same · threshold 0.80");
    expect(JSON.stringify(armed)).not.toContain("fraud");
    expect(JSON.stringify(armed)).not.toContain("score");
    const draft = armed.steps.find((step) => step.kind === "draft");
    expect(draft?.kind === "draft" && draft.lines[0]).toBe("pattern_id: absence.after-until");
    expect(draft?.kind === "draft" && draft.lines).toContain("lookback: 4");
  });

  test("rejects an unknown pattern before a Jev check", () => {
    const rejected = run(
      [
        frame("sentence", { text: policy }),
        frame("draft", { pattern_id: "precedence", parameters: {} }),
        frame("rejected", { reason: "unknown_pattern" }),
      ].join(""),
    );
    expect(rejected.status).toBe("rejected");
    expect(rejected.steps.map((step) => step.kind)).toEqual(["policy", "draft", "catalogue", "rejected"]);
    expect(rejected.steps.some((step) => step.kind === "check")).toBe(false);
    expect(rejected.monitor).toBeNull();
    const catalogue = rejected.steps.find((step) => step.kind === "catalogue");
    expect(catalogue).toEqual({ kind: "catalogue", ok: false, detail: "Unknown pattern" });
  });

  test("records a catalogue parameter failure from the reason code", () => {
    const rejected = run(
      [
        frame("sentence", { text: policy }),
        frame("draft", { pattern_id: "absence.after-until", parameters: { binding_attribute: "bill.id" } }),
        frame("rejected", {
          reason: "catalogue_invalid",
          code: "missing_parameter",
          parameter: "after",
        }),
      ].join(""),
    );
    const last = rejected.steps.at(-1);
    expect(last).toEqual({
      kind: "rejected",
      reason: "catalogue_invalid",
      detail: "Catalogue invalid · missing_parameter · after",
    });
  });

  test("does not treat a missing draft as catalogue success", () => {
    const rejected = run(
      [frame("sentence", { text: "" }), frame("rejected", { reason: "draft_unavailable" })].join(""),
    );
    expect(rejected.steps.map((step) => step.kind)).toEqual(["policy", "rejected"]);
    expect(rejected.draftedBy).toBeNull();
  });

  test("keeps clause marks only when they match the sentence", () => {
    const marked = run(
      frame("sentence", {
        text: "Pay the bill. Keep a credit.",
        clauses: [
          { text: "Pay the bill.", coverage: "covered" },
          { text: "Keep a credit.", coverage: "uncovered" },
        ],
      }),
    );
    expect(policySegments(marked.sentence, marked.clauses)?.map((clause) => clause.coverage)).toEqual([
      "covered",
      "uncovered",
    ]);

    const broken = run(
      frame("sentence", {
        text: "Pay the bill.",
        clauses: [{ text: "Pay", coverage: "yes" }],
      }),
    );
    expect(broken.clauses).toBeNull();
    expect(policySegments("Pay the bill.", [{ text: "Other", coverage: "covered" }])).toBeNull();
  });

  test("a failed connection does not become a rejection", () => {
    const failed = failDraft(beginDraft(composingRun("draft-1"), policy), "Spec authoring is not connected.");
    expect(failed.status).toBe("unavailable");
    expect(failed.steps).toEqual([]);
    expect(failed.error).toBe("Spec authoring is not connected.");
    expect(finishDraft(failed).error).toBe("Spec authoring is not connected.");
    expect(finishDraft(beginDraft(composingRun("draft-1"), policy)).error).toBe(
      "The authoring stream did not send any steps.",
    );
  });
});
