/**
 * Spec authoring view state.
 *
 * Steps come from `ledger-spec`'s SSE frames (`sentence`, `draft`, `check`,
 * `coverage`, `accepted`, `held`, `rejected`). The check frame carries a
 * verdict only. Scores are not invented. The 0.80 figure is the fixed Jev
 * threshold in `ledger-jev` (`SAME_PROBABILITY_THRESHOLD` and
 * `DIFFERENT_PROBABILITY_THRESHOLD`).
 */

export const JEV_THRESHOLD_LABEL = "0.80";

/** Drafter named by the authoring pipeline, recorded only after a draft frame. */
export const DRAFTER = "Gemini";

export type MatchVerdict = "Same" | "Different" | "Unknown";

export type SpecStatus = "composing" | "drafting" | "armed" | "rejected" | "unavailable" | "held";

export type HoldReason = "clause_uncovered" | "clause_unknown";

/** Clause marks from the stream. Absent means coverage was not reported. */
export type ClauseMark = {
  text: string;
  coverage: "covered" | "uncovered" | "unknown";
  /** Jev label stored with the mark. Absent when the frame did not send one. */
  verdict?: MatchVerdict;
};

export type TimelineStep =
  | { kind: "policy"; text: string }
  | { kind: "draft"; patternId: string; lines: string[] }
  | { kind: "catalogue"; ok: boolean; detail: string }
  | { kind: "check"; verdict: MatchVerdict }
  | { kind: "coverage"; clauses: ClauseMark[] }
  | { kind: "armed"; patternId: string; monitor: string; override: boolean }
  | { kind: "held"; reason: HoldReason; detail: string }
  | { kind: "monitor"; patternId: string; monitor: string }
  | { kind: "rejected"; reason: string; detail: string };

export type SpecRun = {
  key: string;
  /** `ledger.authored.<pattern>` once the stream accepts the draft. */
  specId: string | null;
  sentence: string;
  clauses: ClauseMark[] | null;
  patternId: string | null;
  parameters: Record<string, string | number>;
  status: SpecStatus;
  match: MatchVerdict | null;
  rejectReason: string | null;
  monitor: string | null;
  draftedBy: string | null;
  steps: TimelineStep[];
  /** Connection or stream failure. Not a model verdict. */
  error: string | null;
};

export type AuthorEvent =
  | { event: "sentence"; text: string; clauses: ClauseMark[] | null }
  | { event: "draft"; patternId: string; parameters: Record<string, string | number> }
  | { event: "check"; verdict: MatchVerdict }
  | { event: "coverage"; clauses: ClauseMark[] }
  | { event: "accepted"; patternId: string; monitor: string; override: boolean }
  | { event: "held"; reason: HoldReason }
  | {
      event: "rejected";
      reason: string;
      code: string | null;
      parameter: string | null;
      eventKind: string | null;
    };
