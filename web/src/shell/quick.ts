import { agentOf, type InboxSearch } from "../inbox/model";
import { outcomeLabel } from "../stream/reduce";
import type { VerdictItem, VerdictOutcome } from "../stream/types";

const FAULTS = ["double-pay", "no-approval", "exceed-budget", "swapped-vendor", "hidden-memo"] as const;

export type FaultName = (typeof FAULTS)[number];

const VIEWS = {
  dashboard: "/",
  inbox: "/inbox",
  traces: "/traces",
  specs: "/specs",
  verdicts: "/verdicts",
} as const;

export type QuickView = keyof typeof VIEWS;

export type QuickDestination = (typeof VIEWS)[QuickView];

type FilterAction = {
  kind: "filter_inbox";
  verdict?: VerdictOutcome;
  agent?: string;
  spec?: string;
  amount?: "over-1000";
};

type OpenAction = { kind: "open_verdict"; id: string };

type FaultAction = { kind: "inject_fault"; fault: string };

type GoAction = { kind: "go_to"; view: string };

type TextAction = { kind: "text_search" };

type MissingAction = { kind: "missing_target" };

export type QuickAction = FilterAction | OpenAction | FaultAction | GoAction | TextAction | MissingAction;

export type QuickStatus = "routed" | "unknown" | "unsupported" | "missing";

export type QuickExchange = {
  request: unknown;
  response?: unknown;
  failure?: unknown;
};

export type QuickReply = {
  phrase: string;
  status: QuickStatus;
  action: QuickAction;
  exchange: QuickExchange;
};

export type QuickCandidates = {
  agents: string[];
  specs: string[];
  verdicts: string[];
};

export type AppliedQuick =
  | { type: "inbox"; search: InboxSearch; notice: string }
  | { type: "go"; to: QuickDestination; notice: string }
  | { type: "open"; id: string; notice: string }
  | { type: "confirm"; fault: FaultName }
  | { type: "missing"; notice: string }
  | { type: "text"; search: InboxSearch; notice: string };

const OUTCOMES: readonly VerdictOutcome[] = ["Satisfy", "Violate", "Inconclusive"];

export function isFaultName(value: string): value is FaultName {
  return (FAULTS as readonly string[]).includes(value);
}

export function candidatesFrom(verdicts: readonly VerdictItem[]): QuickCandidates {
  return {
    agents: unique(verdicts.map((verdict) => agentOf(verdict)).filter((agent) => agent.length > 0)).slice(0, 8),
    specs: unique(verdicts.map((verdict) => verdict.specId).filter((spec) => spec.length > 0)).slice(0, 8),
    verdicts: verdicts
      .map((verdict) => verdict.id)
      .filter((id) => id.length > 0)
      .slice(0, 12),
  };
}

/** Turn a Jev reply into a palette action. An unknown id is not opened. */
export function applyQuick(reply: QuickReply, knownIds: ReadonlySet<string>): AppliedQuick {
  const action = reply.action;
  if (action.kind === "inject_fault" && reply.status === "routed" && isFaultName(action.fault)) {
    return { type: "confirm", fault: action.fault };
  }
  if (action.kind === "missing_target" || reply.status === "missing") {
    return { type: "missing", notice: "That verdict is not on the stream." };
  }
  if (action.kind === "open_verdict" && reply.status === "routed") {
    if (!knownIds.has(action.id)) {
      return { type: "missing", notice: "That verdict is not on the stream." };
    }
    return { type: "open", id: action.id, notice: "Opened a verdict already on the stream." };
  }
  if (action.kind === "go_to" && reply.status === "routed" && isView(action.view)) {
    return { type: "go", to: VIEWS[action.view], notice: `Opened ${action.view}.` };
  }
  if (action.kind === "filter_inbox" && reply.status === "routed") {
    const search = filterSearch(action);
    if (hasFilter(search)) {
      return { type: "inbox", search, notice: filterNotice(search) };
    }
  }
  return {
    type: "text",
    search: { q: reply.phrase },
    notice:
      reply.status === "unsupported"
        ? "Not a quick action. Showing the phrase as text search."
        : "Jev returned Unknown. Showing the phrase as text search.",
  };
}

export function exchangeLines(reply: QuickReply): string[] {
  const lines: string[] = [`phrase · ${reply.phrase}`];
  if (reply.exchange.failure != null) {
    lines.push(`Jev · Unknown · ${failureLabel(reply.exchange.failure)}`);
  }
  const response = reply.exchange.response;
  if (typeof response !== "object" || response === null || !("answers" in response)) return lines;
  const answers = response.answers;
  if (typeof answers !== "object" || answers === null) return lines;
  for (const [id, answer] of Object.entries(answers)) {
    if (typeof answer !== "object" || answer === null) continue;
    const record = answer as Record<string, unknown>;
    if (record.type === "choice" && typeof record.choice === "string") {
      const probability = probabilityOf(record.probabilities, record.choice);
      lines.push(probability == null ? `${id} · ${record.choice}` : `${id} · ${record.choice} · ${probability.toFixed(2)}`);
    } else if (record.type === "noul" && typeof record.noul === "number") {
      lines.push(`${id} · ${record.noul.toFixed(2)}`);
    }
  }
  return lines;
}

export async function routePhrase(phrase: string, candidates: QuickCandidates): Promise<QuickReply | null> {
  try {
    const response = await fetch("/api/quick-actions", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ phrase, ...candidates }),
    });
    if (!response.ok) return null;
    return parseReply(await response.json());
  } catch {
    return null;
  }
}

function filterSearch(action: FilterAction): InboxSearch {
  return {
    verdict: isOutcome(action.verdict) ? action.verdict : undefined,
    agent: text(action.agent),
    spec: text(action.spec),
    amount: action.amount === "over-1000" ? "over-1000" : undefined,
  };
}

function hasFilter(search: InboxSearch): boolean {
  return Boolean(search.verdict || search.agent || search.spec || search.amount);
}

function filterNotice(search: InboxSearch): string {
  const parts: string[] = [];
  if (search.verdict) parts.push(outcomeLabel(search.verdict));
  if (search.amount === "over-1000") parts.push("Over $1,000");
  if (search.agent) parts.push(search.agent);
  if (search.spec) parts.push(search.spec);
  return `Inbox filters · ${parts.join(" · ")}`;
}

function parseReply(value: unknown): QuickReply | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (typeof record.phrase !== "string" || !isStatus(record.status)) return null;
  const action = parseAction(record.action);
  if (!action) return null;
  const exchange = record.exchange;
  if (typeof exchange !== "object" || exchange === null || !("request" in exchange)) return null;
  return {
    phrase: record.phrase,
    status: record.status,
    action,
    exchange: exchange as QuickExchange,
  };
}

function parseAction(value: unknown): QuickAction | null {
  if (typeof value !== "object" || value === null || !("kind" in value)) return null;
  const record = value as Record<string, unknown>;
  switch (record.kind) {
    case "filter_inbox":
      return {
        kind: "filter_inbox",
        verdict: isOutcome(record.verdict) ? record.verdict : undefined,
        agent: text(record.agent),
        spec: text(record.spec),
        amount: record.amount === "over-1000" ? "over-1000" : undefined,
      };
    case "open_verdict":
      return typeof record.id === "string" && record.id.length > 0 ? { kind: "open_verdict", id: record.id } : null;
    case "inject_fault":
      return typeof record.fault === "string" ? { kind: "inject_fault", fault: record.fault } : null;
    case "go_to":
      return typeof record.view === "string" ? { kind: "go_to", view: record.view } : null;
    case "text_search":
      return { kind: "text_search" };
    case "missing_target":
      return { kind: "missing_target" };
    default:
      return null;
  }
}

function isStatus(value: unknown): value is QuickStatus {
  return value === "routed" || value === "unknown" || value === "unsupported" || value === "missing";
}

function isOutcome(value: unknown): value is VerdictOutcome {
  return typeof value === "string" && OUTCOMES.includes(value as VerdictOutcome);
}

function isView(value: string): value is QuickView {
  return value in VIEWS;
}

function text(value: unknown): string | undefined {
  return typeof value === "string" && value.trim().length > 0 ? value : undefined;
}

function unique(values: string[]): string[] {
  return [...new Set(values)];
}

function probabilityOf(probabilities: unknown, choice: string): number | null {
  if (typeof probabilities !== "object" || probabilities === null || !(choice in probabilities)) return null;
  const probability = (probabilities as Record<string, unknown>)[choice];
  return typeof probability === "number" ? probability : null;
}

function failureLabel(failure: unknown): string {
  if (typeof failure === "string") return failure;
  if (typeof failure === "object" && failure !== null && "http" in failure) return "http";
  return "unknown";
}
