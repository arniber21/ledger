import { outcomeLabel } from "../stream/reduce";
import type { RankScores, SavedView, VerdictItem, VerdictOutcome } from "../stream/types";

export type InboxView = "board" | "list" | "split";

export type TriageStatus = "New" | "Reviewing" | "Confirmed" | "Dismissed";

export type InboxGroupBy = "status" | "verdict" | "agent" | "spec";

export type InboxSort = "newest" | "oldest" | "amount" | "id" | "priority";

/** Slider positions, 0 to 100. Divided by 100 before they weight a score. */
export type RankSliders = {
  money: number;
  evidence: number;
  ambiguity: number;
  intent: number;
};

export type RankWeights = {
  money: number;
  evidence: number;
  ambiguity: number;
  intent: number;
};

export const RANK_TOP_LEVEL = 4;

export const DEFAULT_RANK_SLIDERS: RankSliders = {
  money: 25,
  evidence: 25,
  ambiguity: 25,
  intent: 25,
};

export const DEFAULT_RANK_WEIGHTS: RankWeights = {
  money: 0.25,
  evidence: 0.25,
  ambiguity: 0.25,
  intent: 0.25,
};

const WEIGHTS_KEY = "ledger.inbox.rank-weights";

export type InboxSearch = {
  view?: InboxView;
  group?: InboxGroupBy;
  sort?: InboxSort;
  verdict?: VerdictOutcome;
  status?: TriageStatus;
  agent?: string;
  amount?: "over-1000";
  assignee?: "me";
  jev?: "unknown";
  saved?: SavedView;
  dismissed?: boolean;
  q?: string;
};

export type Triage = {
  verdictId: string;
  status: TriageStatus;
  assignee: string | null;
  updatedAt: string;
  updatedBy: string;
};

export type InboxItem = {
  verdict: VerdictItem;
  status: TriageStatus;
  assignee: string | null;
  agent: string;
};

export type ItemGroup = {
  key: string;
  label: string;
  items: InboxItem[];
};

export type InboxQuery = {
  verdict?: VerdictOutcome;
  status?: TriageStatus;
  agent?: string;
  amount?: "over-1000";
  assignee?: "me";
  jev?: "unknown";
  q?: string;
  dismissedOnly: boolean;
  group: InboxGroupBy;
  sort: InboxSort;
};

export const TRIAGE_STATUSES: readonly TriageStatus[] = ["New", "Reviewing", "Confirmed", "Dismissed"];

export const INBOX_VIEW_KEY = "ledger.inbox.view";

const OVER_1000_CENTS = 100_000;

const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

export function isInboxView(value: unknown): value is InboxView {
  return value === "board" || value === "list" || value === "split";
}

export function isTriageStatus(value: unknown): value is TriageStatus {
  return value === "New" || value === "Reviewing" || value === "Confirmed" || value === "Dismissed";
}

export function isInboxGroup(value: unknown): value is InboxGroupBy {
  return value === "status" || value === "verdict" || value === "agent" || value === "spec";
}

export function isInboxSort(value: unknown): value is InboxSort {
  return value === "newest" || value === "oldest" || value === "amount" || value === "id" || value === "priority";
}

function sliderValue(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return 25;
  return Math.min(100, Math.max(0, Math.round(value)));
}

export function storedRankSliders(): RankSliders {
  try {
    const raw = localStorage.getItem(WEIGHTS_KEY);
    if (!raw) return DEFAULT_RANK_SLIDERS;
    const value = JSON.parse(raw) as Record<string, unknown>;
    return {
      money: sliderValue(value.money),
      evidence: sliderValue(value.evidence),
      ambiguity: sliderValue(value.ambiguity),
      intent: sliderValue(value.intent),
    };
  } catch {
    return DEFAULT_RANK_SLIDERS;
  }
}

export function rememberRankSliders(sliders: RankSliders): void {
  try {
    localStorage.setItem(WEIGHTS_KEY, JSON.stringify(sliders));
  } catch {
    // The sliders still apply for this visit when storage is blocked.
  }
}

export function weightsFromSliders(sliders: RankSliders): RankWeights {
  return {
    money: sliders.money / 100,
    evidence: sliders.evidence / 100,
    ambiguity: sliders.ambiguity / 100,
    intent: sliders.intent / 100,
  };
}

/**
 * Weighted sum of raw scores divided by 4.
 * Any Unknown dimension, or weights that are not a positive finite set, yields null.
 * Null is not a priority of zero.
 */
export function priorityOf(rank: RankScores | undefined, weights: RankWeights): number | null {
  if (!rank) return null;
  const factors = [rank.money, rank.evidence, rank.ambiguity, rank.intent];
  if (factors.some((value) => value == null || !Number.isFinite(value))) return null;
  const parts = [weights.money, weights.evidence, weights.ambiguity, weights.intent];
  if (parts.some((value) => !Number.isFinite(value) || value < 0)) return null;
  if (parts.reduce((sum, value) => sum + value, 0) === 0) return null;
  return factors.reduce<number>((sum, raw, index) => sum + parts[index] * ((raw as number) / RANK_TOP_LEVEL), 0);
}

export function severityRank(outcome: VerdictOutcome): number {
  switch (outcome) {
    case "Violate":
      return 0;
    case "Inconclusive":
      return 1;
    case "Satisfy":
      return 2;
  }
}

export function scoreLabel(rank: RankScores | undefined, value: number | null | undefined): string {
  if (!rank) return "Not stored";
  if (value == null || !Number.isFinite(value)) return "Unknown";
  return value.toFixed(2);
}

export function isUuid(value: string): boolean {
  return UUID_PATTERN.test(value);
}

export function storedInboxView(): InboxView | undefined {
  try {
    const value = localStorage.getItem(INBOX_VIEW_KEY);
    return isInboxView(value) ? value : undefined;
  } catch {
    return undefined;
  }
}

export function rememberInboxView(view: InboxView): void {
  try {
    localStorage.setItem(INBOX_VIEW_KEY, view);
  } catch {
    // The address bar still carries the mode when storage is blocked.
  }
}

function textParam(value: unknown): string | undefined {
  return typeof value === "string" && value.trim().length > 0 ? value : undefined;
}

export function parseInboxSearch(search: Record<string, unknown>): InboxSearch {
  return {
    view: isInboxView(search.view) ? search.view : undefined,
    group: isInboxGroup(search.group) ? search.group : undefined,
    sort: isInboxSort(search.sort) ? search.sort : undefined,
    verdict:
      search.verdict === "Satisfy" || search.verdict === "Violate" || search.verdict === "Inconclusive"
        ? search.verdict
        : undefined,
    status: isTriageStatus(search.status) ? search.status : undefined,
    agent: textParam(search.agent),
    amount: search.amount === "over-1000" ? "over-1000" : undefined,
    assignee: search.assignee === "me" ? "me" : undefined,
    jev: search.jev === "unknown" ? "unknown" : undefined,
    saved:
      search.saved === "assigned" ||
      search.saved === "violations" ||
      search.saved === "unknowns" ||
      search.saved === "over-1000"
        ? search.saved
        : undefined,
    dismissed:
      search.dismissed === true || search.dismissed === "1" || search.dismissed === 1 ? true : undefined,
    q: textParam(search.q),
  };
}

export function savedInboxSearch(id: SavedView): InboxSearch {
  const base: InboxSearch = { view: "list", group: "status", sort: "newest", saved: id };
  switch (id) {
    case "assigned":
      return { ...base, assignee: "me" };
    case "violations":
      return { ...base, view: "board", verdict: "Violate" };
    case "unknowns":
      return { ...base, group: "spec", jev: "unknown" };
    case "over-1000":
      return { ...base, group: "agent", amount: "over-1000" };
  }
}

export function mergeInboxSearch(previous: object, patch: Partial<InboxSearch>): InboxSearch {
  return parseInboxSearch({ ...(previous as Record<string, unknown>), ...patch });
}

export function queryFromSearch(search: InboxSearch): InboxQuery {
  return {
    verdict: search.verdict,
    status: search.status,
    agent: search.agent,
    amount: search.amount,
    assignee: search.assignee,
    jev: search.jev,
    q: search.q,
    dismissedOnly: search.dismissed === true,
    group: search.group ?? "status",
    sort: search.sort ?? "newest",
  };
}

export function hasInboxFilters(search: InboxSearch): boolean {
  return Boolean(
    search.verdict ||
      search.status ||
      search.agent ||
      search.amount ||
      search.assignee ||
      search.jev ||
      search.q ||
      search.saved,
  );
}

export function agentOf(verdict: VerdictItem): string {
  const timeline = verdict.timeline ?? [];
  const agent = timeline.find((step) => step.source.startsWith("agent."));
  return agent?.source ?? "";
}

export function statusOf(verdictId: string, triage: readonly Triage[]): TriageStatus {
  return triage.find((row) => row.verdictId === verdictId)?.status ?? "New";
}

export function toInboxItem(verdict: VerdictItem, triage: readonly Triage[]): InboxItem {
  const row = triage.find((entry) => entry.verdictId === verdict.id);
  const assignee = row?.assignee ?? (verdict.assignedToMe ? "me" : null);
  return {
    verdict,
    status: row?.status ?? "New",
    assignee,
    agent: agentOf(verdict),
  };
}

function haystack(item: InboxItem): string {
  return [item.verdict.id, item.verdict.title, item.verdict.specId, item.agent, item.verdict.evidence]
    .join(" ")
    .toLowerCase();
}

export function matchesQuery(item: InboxItem, query: InboxQuery): boolean {
  if (query.dismissedOnly && item.status !== "Dismissed") return false;
  if (!query.dismissedOnly && query.verdict == null && item.verdict.outcome === "Satisfy") return false;
  if (query.verdict && item.verdict.outcome !== query.verdict) return false;
  if (query.status && item.status !== query.status) return false;
  if (query.agent && item.agent !== query.agent) return false;
  if (query.amount === "over-1000") {
    const cents = item.verdict.amountCents;
    if (cents == null || cents < OVER_1000_CENTS) return false;
  }
  if (query.assignee === "me" && item.assignee !== "me") return false;
  if (query.jev === "unknown" && item.verdict.jevUnknown !== true) return false;
  if (query.q && !haystack(item).includes(query.q.trim().toLowerCase())) return false;
  return true;
}

function timeValue(recordedAt: string): number | null {
  if (!recordedAt) return null;
  const value = Date.parse(recordedAt);
  return Number.isFinite(value) ? value : null;
}

export function compareItems(
  sort: InboxSort,
  left: InboxItem,
  right: InboxItem,
  weights: RankWeights = DEFAULT_RANK_WEIGHTS,
): number {
  switch (sort) {
    case "priority":
      return comparePriority(left, right, weights);
    case "id":
      return left.verdict.id.localeCompare(right.verdict.id);
    case "amount": {
      const leftAmount = left.verdict.amountCents;
      const rightAmount = right.verdict.amountCents;
      if (leftAmount == null && rightAmount == null) return left.verdict.id.localeCompare(right.verdict.id);
      if (leftAmount == null) return 1;
      if (rightAmount == null) return -1;
      return rightAmount - leftAmount || left.verdict.id.localeCompare(right.verdict.id);
    }
    case "oldest":
    case "newest": {
      const leftTime = timeValue(left.verdict.recordedAt);
      const rightTime = timeValue(right.verdict.recordedAt);
      if (leftTime == null && rightTime == null) return left.verdict.id.localeCompare(right.verdict.id);
      if (leftTime == null) return 1;
      if (rightTime == null) return -1;
      const delta = sort === "newest" ? rightTime - leftTime : leftTime - rightTime;
      return delta || left.verdict.id.localeCompare(right.verdict.id);
    }
  }
}

function comparePriority(left: InboxItem, right: InboxItem, weights: RankWeights): number {
  const leftScore = priorityOf(left.verdict.rank, weights);
  const rightScore = priorityOf(right.verdict.rank, weights);
  if (leftScore != null && rightScore != null) {
    if (leftScore !== rightScore) return rightScore - leftScore;
    const severity = severityRank(left.verdict.outcome) - severityRank(right.verdict.outcome);
    if (severity !== 0) return severity;
    return left.verdict.id.localeCompare(right.verdict.id);
  }
  if (leftScore == null && rightScore == null) {
    const severity = severityRank(left.verdict.outcome) - severityRank(right.verdict.outcome);
    if (severity !== 0) return severity;
    return left.verdict.id.localeCompare(right.verdict.id);
  }
  return leftScore == null ? 1 : -1;
}

export function sortItems(
  items: readonly InboxItem[],
  sort: InboxSort,
  weights: RankWeights = DEFAULT_RANK_WEIGHTS,
): InboxItem[] {
  return [...items].sort((left, right) => compareItems(sort, left, right, weights));
}

export function needsReview(item: InboxItem): boolean {
  return (
    (item.verdict.outcome === "Violate" || item.verdict.outcome === "Inconclusive") &&
    (item.status === "New" || item.status === "Reviewing")
  );
}

export function countNeedsReview(
  verdicts: readonly VerdictItem[],
  triage: readonly Triage[],
  reviewed?: ReadonlySet<string>,
): number {
  return verdicts
    .map((verdict) => toInboxItem(verdict, triage))
    .filter((item) => needsReview(item) && !reviewed?.has(item.verdict.id))
    .length;
}

function groupKey(item: InboxItem, group: InboxGroupBy): { key: string; label: string } {
  switch (group) {
    case "status":
      return { key: item.status, label: item.status };
    case "verdict":
      return { key: item.verdict.outcome, label: outcomeLabel(item.verdict.outcome) };
    case "agent":
      return item.agent ? { key: item.agent, label: item.agent } : { key: "", label: "No source" };
    case "spec":
      return item.verdict.specId
        ? { key: item.verdict.specId, label: item.verdict.specId }
        : { key: "", label: "No spec" };
  }
}

function groupRank(group: InboxGroupBy, key: string): number {
  if (group === "status") return TRIAGE_STATUSES.indexOf(key as TriageStatus);
  if (group === "verdict") return ["Violate", "Inconclusive", "Satisfy"].indexOf(key);
  if (key === "") return 1;
  return 0;
}

export function groupItems(items: readonly InboxItem[], group: InboxGroupBy): ItemGroup[] {
  const buckets = new Map<string, ItemGroup>();
  for (const item of items) {
    const { key, label } = groupKey(item, group);
    const existing = buckets.get(key);
    if (existing) existing.items.push(item);
    else buckets.set(key, { key, label, items: [item] });
  }
  return [...buckets.values()].sort((left, right) => {
    const rank = groupRank(group, left.key) - groupRank(group, right.key);
    if (rank !== 0 && groupRank(group, left.key) !== -1 && groupRank(group, right.key) !== -1) return rank;
    return left.label.localeCompare(right.label);
  });
}

export function prepareInbox(
  verdicts: readonly VerdictItem[],
  triage: readonly Triage[],
  query: InboxQuery,
  weights: RankWeights = DEFAULT_RANK_WEIGHTS,
): { items: InboxItem[]; groups: ItemGroup[]; attention: number } {
  const items = sortItems(
    verdicts.map((verdict) => toInboxItem(verdict, triage)).filter((item) => matchesQuery(item, query)),
    query.sort,
    weights,
  );
  return {
    items,
    groups: groupItems(items, query.group),
    attention: countNeedsReview(verdicts, triage),
  };
}

export function orderedIds(
  view: InboxView,
  items: readonly InboxItem[],
  groups: readonly ItemGroup[],
  collapsed: ReadonlySet<string>,
): string[] {
  if (view === "board") {
    return TRIAGE_STATUSES.flatMap((status) =>
      items.filter((item) => item.status === status).map((item) => item.verdict.id),
    );
  }
  if (view === "split") return items.map((item) => item.verdict.id);
  return groups.flatMap((group) =>
    collapsed.has(group.key) ? [] : group.items.map((item) => item.verdict.id),
  );
}

export function countSavedInbox(
  verdicts: readonly VerdictItem[],
  triage: readonly Triage[],
  id: SavedView,
): number {
  const query = queryFromSearch(savedInboxSearch(id));
  return prepareInbox(verdicts, triage, query).items.length;
}

export function ageLabel(recordedAt: string, now: number): string {
  const value = timeValue(recordedAt);
  if (value == null) return "—";
  const seconds = Math.max(0, Math.round((now - value) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `${hours}h`;
  return `${Math.round(hours / 24)}d`;
}

export function amountLabel(cents: number | null): string {
  if (cents == null) return "—";
  return new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" }).format(cents / 100);
}

export function shortId(id: string): string {
  return id.length <= 8 ? id : id.slice(0, 8);
}

export function initials(assignee: string | null): string {
  if (!assignee) return "—";
  if (assignee.toLowerCase() === "me") return "ME";
  const letters = assignee
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase() ?? "")
    .join("");
  return letters || "—";
}

export function lastReviewedLabel(triage: readonly Triage[], now: number): string {
  const reviewed = triage.filter((row) => row.status === "Confirmed" && timeValue(row.updatedAt) != null);
  const latest = reviewed.reduce<Triage | null>((best, row) => {
    if (!best) return row;
    return (timeValue(row.updatedAt) ?? 0) > (timeValue(best.updatedAt) ?? 0) ? row : best;
  }, null);
  if (!latest) return "No verdict has been reviewed.";
  const who = latest.updatedBy ? ` by ${latest.updatedBy}` : "";
  return `Last reviewed ${ageLabel(latest.updatedAt, now)} ago${who}`;
}

export function parseTriagePayload(value: unknown): Triage[] {
  if (typeof value !== "object" || value === null || !("items" in value)) return [];
  const items = (value as { items: unknown }).items;
  if (!Array.isArray(items)) return [];
  const rows: Triage[] = [];
  for (const item of items) {
    if (typeof item !== "object" || item === null) continue;
    const row = item as Record<string, unknown>;
    if (typeof row.verdictId !== "string" || !isTriageStatus(row.status)) continue;
    rows.push({
      verdictId: row.verdictId,
      status: row.status,
      assignee: typeof row.assignee === "string" && row.assignee.length > 0 ? row.assignee : null,
      updatedAt: typeof row.updatedAt === "string" ? row.updatedAt : "",
      updatedBy: typeof row.updatedBy === "string" ? row.updatedBy : "",
    });
  }
  return rows;
}

export type FilterChip = { id: string; label: string; clear: Partial<InboxSearch> };

export function filterChips(search: InboxSearch): FilterChip[] {
  const chips: FilterChip[] = [];
  if (search.verdict) {
    chips.push({
      id: "verdict",
      label: `Verdict · ${outcomeLabel(search.verdict)}`,
      clear: { verdict: undefined, saved: undefined },
    });
  }
  if (search.status) {
    chips.push({
      id: "status",
      label: `Status · ${search.status}`,
      clear: { status: undefined, saved: undefined },
    });
  }
  if (search.agent) {
    chips.push({
      id: "agent",
      label: `Agent · ${search.agent}`,
      clear: { agent: undefined, saved: undefined },
    });
  }
  if (search.amount === "over-1000") {
    chips.push({
      id: "amount",
      label: "Amount · Over $1,000",
      clear: { amount: undefined, saved: undefined },
    });
  }
  if (search.assignee === "me") {
    chips.push({
      id: "assignee",
      label: "Assigned to me",
      clear: { assignee: undefined, saved: undefined },
    });
  }
  if (search.jev === "unknown") {
    chips.push({
      id: "jev",
      label: "Jev unknowns",
      clear: { jev: undefined, saved: undefined },
    });
  }
  return chips;
}
