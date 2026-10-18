import { describe, expect, test } from "bun:test";
import {
  applyAuthorEvents,
  beginDraft,
  checkDetail,
  composingRun,
  coverageSummary,
  drainSse,
  eventsFromSse,
  failDraft,
  finishDraft,
  policySegments,
  scopeOf,
  stepTitle,
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

  test("a two-clause draft that covers one clause stays unarmed", () => {
    const held = run(
      [
        frame("sentence", {
          text: "Pay the bill. Keep a credit.",
          clauses: [
            { text: "Pay the bill. ", coverage: "covered", verdict: "Same" },
            { text: "Keep a credit.", coverage: "uncovered", verdict: "Different" },
          ],
        }),
        frame("draft", {
          pattern_id: "absence.after-until",
          parameters: { binding_attribute: "bill.id" },
        }),
        frame("check", { verdict: "Same" }),
        frame("coverage", {
          clauses: [
            { text: "Pay the bill. ", coverage: "covered", verdict: "Same" },
            { text: "Keep a credit.", coverage: "uncovered", verdict: "Different" },
          ],
        }),
        frame("held", { reason: "clause_uncovered" }),
      ].join(""),
    );
    expect(held.status).toBe("held");
    expect(held.monitor).toBeNull();
    expect(held.specId).toBeNull();
    expect(held.steps.map((step) => step.kind)).toEqual(["policy", "draft", "catalogue", "check", "coverage", "held"]);
    expect(policySegments(held.sentence, held.clauses)?.map((clause) => clause.coverage)).toEqual([
      "covered",
      "uncovered",
    ]);
    expect(coverageSummary(held.clauses)).toBe("1 uncovered");
    const heldStep = held.steps.at(-1);
    expect(heldStep?.kind === "held" && stepTitle(heldStep)).toBe("Needs review");
    expect(finishDraft(held).status).toBe("held");
    expect(JSON.stringify(held)).not.toContain("fraud");
  });

  test("a coverage failure stays unknown until an override arms it", () => {
    const unknown = run(
      [
        frame("sentence", { text: "Pay the bill. Keep a credit." }),
        frame("check", { verdict: "Same" }),
        frame("coverage", {
          clauses: [
            { text: "Pay the bill. ", coverage: "unknown", verdict: "Unknown" },
            { text: "Keep a credit.", coverage: "unknown", verdict: "Unknown" },
          ],
        }),
        frame("held", { reason: "clause_unknown" }),
      ].join(""),
    );
    expect(unknown.status).toBe("held");
    expect(coverageSummary(unknown.clauses)).toBe("Unknown");
    expect(unknown.steps.some((step) => step.kind === "armed")).toBe(false);

    const armed = applyAuthorEvents(beginDraft(unknown, unknown.sentence), [
      ...eventsFromSse(
        [
          frame("check", { verdict: "Same" }),
          frame("coverage", {
            clauses: [
              { text: "Pay the bill. ", coverage: "unknown", verdict: "Unknown" },
              { text: "Keep a credit.", coverage: "unknown", verdict: "Unknown" },
            ],
          }),
          frame("accepted", { pattern_id: "absence.after-until", monitor: "running", override: true }),
        ].join(""),
      ),
    ]);
    expect(armed.status).toBe("armed");
    const armedStep = armed.steps.find((step) => step.kind === "armed");
    expect(armedStep?.kind === "armed" && stepTitle(armedStep)).toBe("Armed with override");
  });

  test("a fully covered draft still arms", () => {
    const armed = run(
      [
        frame("sentence", { text: "Pay the bill. Keep a credit." }),
        frame("draft", { pattern_id: "absence.after-until", parameters: { binding_attribute: "bill.id" } }),
        frame("check", { verdict: "Same" }),
        frame("coverage", {
          clauses: [
            { text: "Pay the bill. ", coverage: "covered", verdict: "Same" },
            { text: "Keep a credit.", coverage: "covered", verdict: "Same" },
          ],
        }),
        frame("accepted", { pattern_id: "absence.after-until", monitor: "running", override: false }),
      ].join(""),
    );
    expect(armed.status).toBe("armed");
    expect(armed.specId).toBe("ledger.authored.absence.after-until");
    expect(coverageSummary(policySegments(armed.sentence, armed.clauses))).toBe("Covered");
    const armedStep = armed.steps.find((step) => step.kind === "armed");
    expect(armedStep?.kind === "armed" && armedStep.override).toBe(false);
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

import { composingRun as _c, beginDraft as _b, reopenDraft, stepperOf, whyNotArmed } from "./reduce";

describe("authoring stepper", () => {
  test("composing has no stepper, drafting starts at step one", () => {
    expect(stepperOf(_c("k"))).toBeNull();
    expect(stepperOf(_b(_c("k"), "Pay once"))?.current).toBe(0);
  });
  test("retry keeps the text and returns to the editor", () => {
    const reopened = reopenDraft({ ..._b(_c("k"), "Pay once"), status: "rejected" });
    expect(reopened.status).toBe("composing");
    expect(reopened.sentence).toBe("Pay once");
  });
  test("an unfinished draft has no why-not", () => {
    expect(whyNotArmed(_b(_c("k"), "x"))).toBeNull();
    expect(whyNotArmed({ ..._c("k"), status: "held" })).toContain("Inconclusive");
  });
});
