import {
  DRAFTER,
  JEV_THRESHOLD_LABEL,
  type AuthorEvent,
  type ClauseMark,
  type HoldReason,
  type MatchVerdict,
  type SpecRun,
  type TimelineStep,
} from "./types";

const HOLD_DETAIL: Record<HoldReason, string> = {
  clause_uncovered: "A clause is uncovered. Arming needs an override.",
  clause_unknown: "A clause is unknown. Arming needs an override.",
};

const REASON_LABEL: Record<string, string> = {
  empty_policy: "Empty policy",
  draft_unavailable: "Draft unavailable",
  draft_unreadable: "Draft unreadable",
  unknown_pattern: "Unknown pattern",
  catalogue_invalid: "Catalogue invalid",
  draft_differs: "Draft differs",
  match_inconclusive: "Match inconclusive",
  monitor_failed: "Monitor failed",
};

export function composingRun(key: string): SpecRun {
  return {
    key,
    specId: null,
    sentence: "",
    clauses: null,
    patternId: null,
    parameters: {},
    status: "composing",
    match: null,
    rejectReason: null,
    monitor: null,
    draftedBy: null,
    steps: [],
    error: null,
  };
}

export function beginDraft(run: SpecRun, sentence: string): SpecRun {
  return {
    ...composingRun(run.key),
    sentence,
    status: "drafting",
  };
}

export function failDraft(run: SpecRun, error: string): SpecRun {
  if (run.status === "armed" || run.status === "rejected" || run.status === "held") return run;
  return { ...run, status: "unavailable", error };
}

/** Closes a stream that ended without an accept or reject frame. */
export function finishDraft(run: SpecRun): SpecRun {
  if (run.status === "armed" || run.status === "rejected" || run.status === "unavailable" || run.status === "held") {
    return run;
  }
  if (run.steps.some((step) => step.kind === "rejected" || step.kind === "armed" || step.kind === "monitor")) {
    return run;
  }
  if (run.steps.length === 0) {
    return failDraft(run, "The authoring stream did not send any steps.");
  }
  return failDraft(run, "The authoring stream ended before accepting or rejecting the draft.");
}

export function scopeOf(parameters: Record<string, string | number>): string | null {
  const binding = parameters.binding_attribute;
  return typeof binding === "string" && binding.length > 0 ? binding : null;
}

export function checkDetail(verdict: MatchVerdict): string {
  return `${verdict} · threshold ${JEV_THRESHOLD_LABEL}`;
}

export function stepTitle(step: TimelineStep): string {
  switch (step.kind) {
    case "policy":
      return "Policy entered";
    case "draft":
      return "Gemini drafted a pattern";
    case "catalogue":
      return step.ok ? "Catalogue validated" : "Catalogue rejected";
    case "check":
      return "Jev checked the draft";
    case "coverage":
      return "Clause coverage";
    case "armed":
      return step.override ? "Armed with override" : "Armed on the live stream";
    case "held":
      return "Needs review";
    case "monitor":
      return "Monitor reported";
    case "rejected":
      return "Not accepted";
  }
}

export function draftLines(patternId: string, parameters: Record<string, string | number>): string[] {
  const lines = [`pattern_id: ${patternId}`];
  for (const [name, value] of Object.entries(parameters)) {
    lines.push(`${name}: ${value}`);
  }
  return lines;
}

/**
 * Returns clause marks only when they reconstruct the sentence.
 * A missing or mismatched report stays unmarked.
 */
export function coverageWord(coverage: ClauseMark["coverage"]): string {
  switch (coverage) {
    case "covered":
      return "Covered";
    case "uncovered":
      return "Uncovered";
    case "unknown":
      return "Unknown";
  }
}

/** One line for the spec properties. Null clauses were not reported. */
export function coverageSummary(clauses: ClauseMark[] | null): string {
  if (!clauses || clauses.length === 0) return "Not reported";
  const uncovered = clauses.filter((clause) => clause.coverage === "uncovered").length;
  const unknown = clauses.filter((clause) => clause.coverage === "unknown").length;
  if (uncovered === 0 && unknown === 0) return "Covered";
  if (uncovered > 0 && unknown > 0) return `${uncovered} uncovered · ${unknown} unknown`;
  if (uncovered === 1) return "1 uncovered";
  if (uncovered > 1) return `${uncovered} uncovered`;
  if (unknown === clauses.length) return "Unknown";
  return unknown === 1 ? "1 unknown" : `${unknown} unknown`;
}

export function policySegments(sentence: string, clauses: ClauseMark[] | null): ClauseMark[] | null {
  if (!clauses || clauses.length === 0) return null;
  const tight = clauses.map((clause) => clause.text).join("");
  const spaced = clauses.map((clause) => clause.text).join(" ");
  if (tight !== sentence && spaced !== sentence) return null;
  return clauses;
}

export function rejectDetail(event: Pick<AuthorEvent & { event: "rejected" }, "reason" | "code" | "parameter" | "eventKind">): string {
  const label = REASON_LABEL[event.reason] ?? event.reason;
  const extra = [event.code, event.parameter, event.eventKind].filter((part): part is string => part != null && part.length > 0);
  return extra.length === 0 ? label : `${label} · ${extra.join(" · ")}`;
}

export function parseSseBlock(block: string): AuthorEvent | null {
  let name = "";
  const data: string[] = [];
  for (const raw of block.split("\n")) {
    const line = raw.trim();
    if (line.startsWith("event:")) name = line.slice("event:".length).trim();
    else if (line.startsWith("data:")) data.push(line.slice("data:".length).trim());
  }
  if (!name || data.length === 0) return null;
  return parseAuthorData(name, data.join("\n"));
}

export function drainSse(buffer: string): { events: AuthorEvent[]; rest: string } {
  const parts = buffer.replace(/\r\n/g, "\n").split("\n\n");
  const rest = parts.pop() ?? "";
  const events: AuthorEvent[] = [];
  for (const part of parts) {
    const event = parseSseBlock(part);
    if (event) events.push(event);
  }
  return { events, rest };
}

export function finishSse(rest: string): AuthorEvent[] {
  const event = parseSseBlock(rest);
  return event ? [event] : [];
}

export function eventsFromSse(text: string): AuthorEvent[] {
  const drained = drainSse(text);
  return [...drained.events, ...finishSse(drained.rest)];
}

export function applyAuthorEvent(run: SpecRun, event: AuthorEvent): SpecRun {
  switch (event.event) {
    case "sentence":
      return applySentence(run, event);
    case "draft":
      return {
        ...run,
        patternId: event.patternId,
        parameters: event.parameters,
        draftedBy: DRAFTER,
        steps: [
          ...run.steps,
          { kind: "draft", patternId: event.patternId, lines: draftLines(event.patternId, event.parameters) },
        ],
      };
    case "check":
      return applyCheck(run, event.verdict);
    case "coverage":
      return applyCoverage(run, event.clauses);
    case "accepted":
      return applyAccepted(run, event.patternId, event.monitor, event.override);
    case "held":
      return applyHeld(run, event.reason);
    case "rejected":
      return applyRejected(run, event);
  }
}

export function applyAuthorEvents(run: SpecRun, events: readonly AuthorEvent[]): SpecRun {
  return events.reduce(applyAuthorEvent, run);
}

function applySentence(run: SpecRun, event: Extract<AuthorEvent, { event: "sentence" }>): SpecRun {
  const policy: TimelineStep = { kind: "policy", text: event.text };
  const steps = run.steps.some((step) => step.kind === "policy")
    ? run.steps.map((step) => (step.kind === "policy" ? policy : step))
    : [...run.steps, policy];
  return { ...run, sentence: event.text, clauses: event.clauses, steps };
}

function applyCheck(run: SpecRun, verdict: MatchVerdict): SpecRun {
  const steps = run.steps.some((step) => step.kind === "catalogue")
    ? run.steps
    : [...run.steps, catalogueStep(true, "")];
  return {
    ...run,
    match: verdict,
    steps: [...steps, { kind: "check", verdict }],
  };
}

function applyCoverage(run: SpecRun, clauses: ClauseMark[]): SpecRun {
  return {
    ...run,
    clauses,
    steps: [...run.steps, { kind: "coverage", clauses }],
  };
}

function applyHeld(run: SpecRun, reason: HoldReason): SpecRun {
  return {
    ...run,
    status: "held",
    error: null,
    monitor: null,
    steps: [...run.steps, { kind: "held", reason, detail: HOLD_DETAIL[reason] }],
  };
}

function applyAccepted(run: SpecRun, patternId: string, monitor: string, override: boolean): SpecRun {
  const running = monitor === "running";
  return {
    ...run,
    patternId,
    monitor,
    specId: `ledger.authored.${patternId}`,
    status: running ? "armed" : "unavailable",
    error: running ? null : "The authoring stream did not arm a monitor.",
    steps: [
      ...run.steps,
      running
        ? { kind: "armed", patternId, monitor, override }
        : { kind: "monitor", patternId, monitor },
    ],
  };
}

function applyRejected(run: SpecRun, event: Extract<AuthorEvent, { event: "rejected" }>): SpecRun {
  let steps = run.steps;
  if (!steps.some((step) => step.kind === "catalogue") && isCatalogueReason(event.reason)) {
    const ok = event.reason === "monitor_failed";
    steps = [...steps, catalogueStep(ok, ok ? "" : rejectDetail(event))];
  }
  return {
    ...run,
    status: "rejected",
    rejectReason: event.reason,
    error: null,
    steps: [...steps, { kind: "rejected", reason: event.reason, detail: rejectDetail(event) }],
  };
}

function catalogueStep(ok: boolean, detail: string): TimelineStep {
  return { kind: "catalogue", ok, detail };
}

function isCatalogueReason(reason: string): boolean {
  return reason === "catalogue_invalid" || reason === "unknown_pattern" || reason === "monitor_failed";
}

function parseAuthorData(event: string, data: string): AuthorEvent | null {
  let body: unknown;
  try {
    body = JSON.parse(data);
  } catch {
    return null;
  }
  if (!body || typeof body !== "object" || Array.isArray(body)) return null;
  const record = body as Record<string, unknown>;
  switch (event) {
    case "sentence":
      return typeof record.text === "string"
        ? { event: "sentence", text: record.text, clauses: parseClauses(record.clauses) }
        : null;
    case "draft": {
      if (typeof record.pattern_id !== "string" || record.pattern_id.length === 0) return null;
      const parameters = parseParameters(record.parameters);
      if (!parameters) return null;
      return { event: "draft", patternId: record.pattern_id, parameters };
    }
    case "check":
      return isVerdict(record.verdict) ? { event: "check", verdict: record.verdict } : null;
    case "coverage": {
      const clauses = parseClauses(record.clauses);
      if (!clauses || clauses.length === 0) return null;
      return { event: "coverage", clauses };
    }
    case "accepted":
      return typeof record.pattern_id === "string" && typeof record.monitor === "string"
        ? {
            event: "accepted",
            patternId: record.pattern_id,
            monitor: record.monitor,
            override: record.override === true,
          }
        : null;
    case "held":
      return isHoldReason(record.reason) ? { event: "held", reason: record.reason } : null;
    case "rejected":
      return typeof record.reason === "string" && record.reason.length > 0
        ? {
            event: "rejected",
            reason: record.reason,
            code: stringOrNull(record.code),
            parameter: stringOrNull(record.parameter),
            eventKind: stringOrNull(record.event_kind),
          }
        : null;
    default:
      return null;
  }
}

function parseParameters(value: unknown): Record<string, string | number> | null {
  if (value == null) return {};
  if (typeof value !== "object" || Array.isArray(value)) return null;
  const parameters: Record<string, string | number> = {};
  for (const [name, item] of Object.entries(value)) {
    if (typeof item === "string" || typeof item === "number") parameters[name] = item;
    else return null;
  }
  return parameters;
}

function parseClauses(value: unknown): ClauseMark[] | null {
  if (value == null) return null;
  if (!Array.isArray(value)) return null;
  const clauses: ClauseMark[] = [];
  for (const item of value) {
    if (!item || typeof item !== "object" || Array.isArray(item)) return null;
    const record = item as Record<string, unknown>;
    if (typeof record.text !== "string") return null;
    if (record.coverage !== "covered" && record.coverage !== "uncovered" && record.coverage !== "unknown") {
      return null;
    }
    if (record.verdict != null && !isVerdict(record.verdict)) return null;
    clauses.push({
      text: record.text,
      coverage: record.coverage,
      verdict: isVerdict(record.verdict) ? record.verdict : undefined,
    });
  }
  return clauses;
}

function isVerdict(value: unknown): value is MatchVerdict {
  return value === "Same" || value === "Different" || value === "Unknown";
}

function isHoldReason(value: unknown): value is HoldReason {
  return value === "clause_uncovered" || value === "clause_unknown";
}

function stringOrNull(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}
