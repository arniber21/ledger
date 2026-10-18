import {
  columnFilteringFeature,
  createFilteredRowModel,
  tableFeatures,
  useTable,
  type ColumnDef,
} from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useNavigate } from "@tanstack/react-router";
import { createContext, useContext, useEffect, useMemo, useRef, useState, type DragEvent, type ReactNode, type RefObject } from "react";
import { outcomeLabel } from "../stream/reduce";
import type { BindingLine } from "../stream/types";
import { useShell } from "../shell/context";
import { Header } from "../shell/header";
import { BoardIcon, CheckIcon, FilterIcon, InboxIcon, ListIcon, SplitIcon } from "../shell/icons";
import { inboxBridge } from "./bridge";
import {
  TRIAGE_STATUSES,
  ageLabel,
  agentOf,
  amountLabel,
  bindingLabel,
  cardSubtitle,
  cardTitle,
  DEFAULT_RANK_WEIGHTS,
  jevChip,
  sourceLabel,
  filterChips,
  groupItems,
  hasInboxFilters,
  initials,
  isUuid,
  lastReviewedLabel,
  orderedIds,
  prepareInbox,
  queryFromSearch,
  rememberRankSliders,
  scoreLabel,
  storedRankSliders,
  weightsFromSliders,
  mergeInboxSearch,
  rememberInboxView,
  shortId,
  storedInboxView,
  type RankSliders,
  type InboxGroupBy,
  type InboxItem,
  type InboxSearch,
  type InboxView,
  type ItemGroup,
  type TriageStatus,
} from "./model";
import { useSetTriage, useTriage } from "./use-triage";

const features = tableFeatures({
  columnFilteringFeature,
  filteredRowModel: createFilteredRowModel(),
});

type Features = typeof features;

const columns: ColumnDef<Features, InboxItem>[] = [
  { id: "severity", accessorFn: (row) => row.verdict.outcome, header: "Severity" },
  { id: "id", accessorFn: (row) => row.verdict.id, header: "Id" },
  { id: "title", accessorFn: (row) => cardTitle(row.verdict), header: "Title" },
  { id: "spec", accessorFn: (row) => row.verdict.specId, header: "Spec" },
  { id: "agent", accessorFn: (row) => row.agent, header: "Agent" },
  { id: "amount", accessorFn: (row) => row.verdict.amountCents, header: "Amount" },
  { id: "age", accessorFn: (row) => row.verdict.recordedAt, header: "Age" },
  { id: "assignee", accessorFn: (row) => row.assignee ?? "", header: "Assignee" },
];

const VIEW_ICONS: Record<InboxView, ReactNode> = {
  board: <BoardIcon />,
  list: <ListIcon />,
  split: <SplitIcon />,
};

const WeightsContext = createContext(DEFAULT_RANK_WEIGHTS);

export function InboxPage({ search }: { search: InboxSearch }) {
  const navigate = useNavigate();
  const { metrics, openPalette, setInboxView, selectedId, setSelectedId, registerFeed } = useShell();
  const triage = useTriage();
  const setTriage = useSetTriage();
  const query = queryFromSearch(search);
  const [sliders, setSliders] = useState<RankSliders>(() => storedRankSliders());
  const [weightsOpen, setWeightsOpen] = useState(false);
  const weights = useMemo(() => weightsFromSliders(sliders), [sliders]);
  const prepared = useMemo(
    () => prepareInbox(metrics.verdicts, triage, query, weights),
    [metrics.verdicts, triage, query, weights],
  );
  const view = search.view ?? storedInboxView() ?? "board";
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set());
  const [adding, setAdding] = useState(false);
  const [assignOpen, setAssignOpen] = useState(false);
  const [assignQuery, setAssignQuery] = useState("");
  const [anchor, setAnchor] = useState({ top: 120, left: 280 });
  const [dropStatus, setDropStatus] = useState<TriageStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  const addRef = useRef<HTMLDivElement>(null);
  const weightsRef = useRef<HTMLDivElement>(null);
  const now = Date.now();

  const items = prepared.items;
  const groups = prepared.groups;
  const active = items.find((item) => item.verdict.id === selectedId) ?? null;
  const calm = prepared.attention === 0 && !search.dismissed && !hasInboxFilters(search);
  const feedKey = orderedIds(view, items, groups, collapsed).join("|");

  const agents = useMemo(() => {
    const names = new Set<string>();
    for (const verdict of metrics.verdicts) {
      const agent = agentOf(verdict);
      if (agent) names.add(agent);
    }
    return [...names].sort();
  }, [metrics.verdicts]);

  useEffect(() => {
    if (search.view) return;
    const next = storedInboxView() ?? "board";
    void navigate({
      to: "/inbox",
      search: (previous) => mergeInboxSearch(previous, { view: next }),
      replace: true,
    });
  }, [navigate, search.view]);

  useEffect(() => {
    registerFeed(feedKey.length === 0 ? [] : feedKey.split("|"));
    return () => registerFeed([]);
  }, [feedKey, registerFeed]);

  useEffect(() => {
    if (!statusError) return;
    const timer = window.setTimeout(() => setStatusError(null), 4000);
    return () => window.clearTimeout(timer);
  }, [statusError]);

  useEffect(() => {
    if (!selectedId) return;
    document.querySelector(`[data-inbox-id="${CSS.escape(selectedId)}"]`)?.scrollIntoView({ block: "nearest" });
  }, [selectedId, view]);

  useEffect(() => {
    if (!adding && !weightsOpen) return;
    const onPointer = (event: PointerEvent) => {
      if (adding && !addRef.current?.contains(event.target as Node)) setAdding(false);
      if (weightsOpen && !weightsRef.current?.contains(event.target as Node)) setWeightsOpen(false);
    };
    window.addEventListener("pointerdown", onPointer);
    return () => window.removeEventListener("pointerdown", onPointer);
  }, [adding, weightsOpen]);

  function onWeight(key: keyof RankSliders, value: number) {
    setSliders((current) => {
      const next = { ...current, [key]: value };
      rememberRankSliders(next);
      return next;
    });
    if (search.sort !== "priority") patchSearch({ sort: "priority" });
  }

  function patchSearch(patch: Partial<InboxSearch>) {
    void navigate({ to: "/inbox", search: (previous) => mergeInboxSearch(previous, patch) });
  }

  function selectView(next: InboxView) {
    rememberInboxView(next);
    setInboxView(next);
    patchSearch({ view: next });
  }

  function write(item: InboxItem, status: TriageStatus, assignee: string | null) {
    if (!isUuid(item.verdict.id)) {
      setStatusError("This verdict is not in the store, so its status was not saved.");
      return;
    }
    setTriage.mutate(
      { verdictId: item.verdict.id, status, assignee },
      {
        onError: (error) => {
          setStatusError(error instanceof Error ? error.message : "Status was not saved.");
        },
      },
    );
  }

  function markReviewed() {
    if (!active) return;
    write(active, "Confirmed", active.assignee);
  }

  function dismiss() {
    if (!active) return;
    write(active, "Dismissed", active.assignee);
  }

  function assignTo(assignee: string | null) {
    if (!active) return;
    write(active, active.status, assignee);
    setAssignOpen(false);
    setAssignQuery("");
  }

  function openAssignAt(item: InboxItem, node: HTMLElement) {
    setSelectedId(item.verdict.id);
    const rect = node.getBoundingClientRect();
    setAnchor({ top: rect.bottom + 6, left: Math.min(rect.left, window.innerWidth - 260) });
    setAssignQuery("");
    setAssignOpen(true);
  }

  function openAssign() {
    if (!active) return;
    const node = document.querySelector(`[data-assignee="${CSS.escape(active.verdict.id)}"]`);
    if (node instanceof HTMLElement) openAssignAt(active, node);
    else {
      setAnchor({ top: 120, left: 280 });
      setAssignQuery("");
      setAssignOpen(true);
    }
  }

  function onAssignKey(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      setAssignOpen(false);
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      const queryText = assignQuery.trim().toLowerCase();
      assignTo(queryText.length === 0 || "me".includes(queryText) ? "me" : null);
      return;
    }
    if (event.key === "0") {
      event.preventDefault();
      assignTo(null);
      return;
    }
    if (event.key === "1") {
      event.preventDefault();
      assignTo("me");
    }
  }

  // The bridge is restored in the effect. A render-only write is cleared by
  // Strict Mode's simulated unmount, and Esc back to the inbox would then ignore e, d, and a.
  useEffect(() => {
    inboxBridge.current = {
      assignOpen,
      onAssignKey,
      markReviewed,
      dismiss,
      openAssign,
      focusFilter: () => filterRef.current?.focus(),
    };
    return () => {
      inboxBridge.current = null;
    };
  });

  const meta = calm ? "Nothing needs attention" : `${prepared.attention} need attention`;
  const chips = filterChips(search);
  const showMe = assignQuery.trim().length === 0 || "me".includes(assignQuery.trim().toLowerCase());

  return (
    <WeightsContext.Provider value={weights}>
      <Header
        icon={<InboxIcon />}
        title="Inbox"
        meta={meta}
        actions={
          <div className="inbox-tools">
            <button type="button" className="tool" onClick={() => filterRef.current?.focus()}>
              <FilterIcon />
              Filter
            </button>
            <div className="views" role="group" aria-label="Inbox view" data-testid="view-switcher">
              {(["board", "list", "split"] as const).map((id) => (
                <button
                  key={id}
                  type="button"
                  data-testid={`view-${id}`}
                  aria-pressed={view === id}
                  onClick={() => selectView(id)}
                >
                  {VIEW_ICONS[id]}
                  {id === "board" ? "Board" : id === "list" ? "List" : "Split"}
                </button>
              ))}
            </div>
          </div>
        }
      />
      {statusError ? (
        <p className="notice" role="status">
          {statusError}
        </p>
      ) : null}
      <div className="content content-fill inbox-page" data-testid="inbox-mode" data-view={view}>
        {calm ? (
          <CalmEmpty
            lastReviewed={lastReviewedLabel(triage, now)}
            onInject={openPalette}
            onDismissed={() => patchSearch({ dismissed: true, saved: undefined })}
          />
        ) : (
          <>
            <InboxToolbar
              search={search}
              adding={adding}
              addRef={addRef}
              filterRef={filterRef}
              weightsRef={weightsRef}
              weightsOpen={weightsOpen}
              sliders={sliders}
              agents={agents}
              chips={chips}
              onPatch={patchSearch}
              onToggleAdd={() => setAdding((open) => !open)}
              onCloseAdd={() => setAdding(false)}
              onToggleWeights={() => setWeightsOpen((open) => !open)}
              onWeight={onWeight}
            />
            {items.length === 0 ? (
              <div className="empty" data-testid="inbox-empty">
                <h2>{search.dismissed ? "No dismissed verdicts" : "No verdicts match these filters"}</h2>
                <p>
                  {search.dismissed
                    ? "Dismissed verdicts show up here after a status change is stored."
                    : "Clear a filter to see the rest of the queue."}
                </p>
                {search.dismissed ? (
                  <button type="button" className="tool" onClick={() => patchSearch({ dismissed: undefined })}>
                    Back to inbox
                  </button>
                ) : null}
              </div>
            ) : view === "board" ? (
              <Board
                items={items}
                selectedId={selectedId}
                dropStatus={dropStatus}
                now={now}
                onSelect={setSelectedId}
                onOpenAssign={openAssignAt}
                onDropStatus={setDropStatus}
                onDrop={(status, id) => {
                  const item = items.find((entry) => entry.verdict.id === id);
                  if (!item || item.status === status) return;
                  write(item, status, item.assignee);
                }}
              />
            ) : view === "list" ? (
              <InboxList
                items={items}
                group={query.group}
                collapsed={collapsed}
                selectedId={selectedId}
                now={now}
                onToggle={(key) => {
                  setCollapsed((current) => {
                    const next = new Set(current);
                    if (next.has(key)) next.delete(key);
                    else next.add(key);
                    return next;
                  });
                }}
                onSelect={setSelectedId}
                onOpenAssign={openAssignAt}
              />
            ) : (
              <Split
                items={items}
                selected={active}
                now={now}
                onSelect={setSelectedId}
                onOpenAssign={openAssignAt}
                onReview={markReviewed}
                onDismiss={dismiss}
              />
            )}
          </>
        )}
      </div>
      <footer className="hintbar">
        <span>J and K move · Enter opens · E marks reviewed · D dismisses · A assigns</span>
        <span>1, 2, 3 switch view · / filters</span>
      </footer>
      {assignOpen && active ? (
        <div
          className="assign-menu"
          role="dialog"
          aria-label="Assign"
          data-testid="assign-menu"
          style={{ top: anchor.top, left: anchor.left }}
        >
          <input
            className="inbox-query"
            value={assignQuery}
            placeholder="Search people"
            aria-label="Search people"
            onChange={(event) => setAssignQuery(event.target.value)}
          />
          <button type="button" onClick={() => assignTo(null)}>
            <span className="avatar">—</span>
            <span>Unassigned</span>
            <kbd>0</kbd>
          </button>
          {showMe ? (
            <button type="button" onClick={() => assignTo("me")}>
              <span className="avatar">ME</span>
              <span>Me</span>
              <kbd>1</kbd>
            </button>
          ) : (
            <p className="assign-empty">No one matches</p>
          )}
        </div>
      ) : null}
    </WeightsContext.Provider>
  );
}

function CalmEmpty({
  lastReviewed,
  onInject,
  onDismissed,
}: {
  lastReviewed: string;
  onInject: () => void;
  onDismissed: () => void;
}) {
  return (
    <div className="empty" data-testid="inbox-empty">
      <div className="empty-mark" aria-hidden="true">
        <CheckIcon />
      </div>
      <h2>Nothing needs review</h2>
      <p>Violate and inconclusive verdicts show up here when the stream sends them. A violate is needs review.</p>
      <div className="empty-actions">
        <button type="button" className="inject" onClick={onInject}>
          Inject a fault
        </button>
        <button type="button" className="tool" onClick={onDismissed}>
          View dismissed
        </button>
      </div>
      <p className="empty-meta">{lastReviewed}</p>
    </div>
  );
}

function InboxToolbar({
  search,
  adding,
  addRef,
  filterRef,
  weightsRef,
  weightsOpen,
  sliders,
  agents,
  chips,
  onPatch,
  onToggleAdd,
  onCloseAdd,
  onToggleWeights,
  onWeight,
}: {
  search: InboxSearch;
  adding: boolean;
  addRef: RefObject<HTMLDivElement | null>;
  filterRef: RefObject<HTMLInputElement | null>;
  weightsRef: RefObject<HTMLDivElement | null>;
  weightsOpen: boolean;
  sliders: RankSliders;
  agents: readonly string[];
  chips: { id: string; label: string; clear: Partial<InboxSearch> }[];
  onPatch: (patch: Partial<InboxSearch>) => void;
  onToggleAdd: () => void;
  onCloseAdd: () => void;
  onToggleWeights: () => void;
  onWeight: (key: keyof RankSliders, value: number) => void;
}) {
  return (
    <div className="inbox-toolbar">
      <input
        ref={filterRef}
        className="inbox-query"
        data-testid="inbox-filter"
        aria-label="Filter inbox"
        placeholder="Filter"
        value={search.q ?? ""}
        onChange={(event) => onPatch({ q: event.target.value || undefined, saved: undefined })}
      />
      <label className="inbox-select">
        <span>Group</span>
        <select
          data-testid="inbox-group"
          aria-label="Group by"
          value={search.group ?? "status"}
          onChange={(event) => onPatch({ group: event.target.value as InboxSearch["group"], saved: undefined })}
        >
          <option value="status">Status</option>
          <option value="verdict">Verdict</option>
          <option value="agent">Agent</option>
          <option value="spec">Spec</option>
        </select>
      </label>
      <label className="inbox-select">
        <span>Sort</span>
        <select
          data-testid="inbox-sort"
          aria-label="Sort"
          value={search.sort ?? "newest"}
          onChange={(event) => onPatch({ sort: event.target.value as InboxSearch["sort"], saved: undefined })}
        >
          <option value="newest">Newest</option>
          <option value="oldest">Oldest</option>
          <option value="amount">Amount</option>
          <option value="id">Id</option>
          <option value="priority">Sort by priority</option>
        </select>
      </label>
      <div className="toolbar-add" ref={weightsRef}>
        <button
          type="button"
          className="tool"
          data-testid="rank-weights"
          aria-expanded={weightsOpen}
          onClick={onToggleWeights}
        >
          Weights
        </button>
        {weightsOpen ? (
          <div className="weights-pop" role="dialog" aria-label="Priority weights" data-testid="rank-weights-popover">
            <p className="assign-empty">Weights recombine the stored scores. They do not call Jev.</p>
            <WeightSlider label="Money at stake" testId="rank-weight-money" value={sliders.money} onChange={(value) => onWeight("money", value)} />
            <WeightSlider label="Evidence" testId="rank-weight-evidence" value={sliders.evidence} onChange={(value) => onWeight("evidence", value)} />
            <WeightSlider label="Binding ambiguity" testId="rank-weight-ambiguity" value={sliders.ambiguity} onChange={(value) => onWeight("ambiguity", value)} />
            <WeightSlider label="Repeated action" testId="rank-weight-intent" value={sliders.intent} onChange={(value) => onWeight("intent", value)} />
          </div>
        ) : null}
      </div>
      {chips.map((chip) => (
        <button key={chip.id} type="button" className="chip" data-testid={`inbox-chip-${chip.id}`} onClick={() => onPatch(chip.clear)}>
          {chip.label}
          <span aria-hidden="true">×</span>
        </button>
      ))}
      <div className="toolbar-add" ref={addRef}>
        <button type="button" className="tool" data-testid="inbox-add-filter" aria-expanded={adding} onClick={onToggleAdd}>
          + Filter
        </button>
        {adding ? (
          <div className="filter-pop" role="menu">
            <button type="button" role="menuitem" onClick={() => { onPatch({ verdict: "Violate", saved: undefined }); onCloseAdd(); }}>
              Verdict · Needs review
            </button>
            <button type="button" role="menuitem" onClick={() => { onPatch({ verdict: "Inconclusive", saved: undefined }); onCloseAdd(); }}>
              Verdict · Inconclusive
            </button>
            <button type="button" role="menuitem" onClick={() => { onPatch({ verdict: "Satisfy", saved: undefined }); onCloseAdd(); }}>
              Verdict · Satisfied
            </button>
            {TRIAGE_STATUSES.map((status) => (
              <button
                key={status}
                type="button"
                role="menuitem"
                onClick={() => {
                  onPatch({ status, saved: undefined });
                  onCloseAdd();
                }}
              >
                Status · {status}
              </button>
            ))}
            <button type="button" role="menuitem" onClick={() => { onPatch({ amount: "over-1000", saved: undefined }); onCloseAdd(); }}>
              Amount · Over $1,000
            </button>
            {agents.length === 0 ? (
              <p className="assign-empty">No agent on the stream</p>
            ) : (
              agents.map((agent) => (
                <button
                  key={agent}
                  type="button"
                  role="menuitem"
                  onClick={() => {
                    onPatch({ agent, saved: undefined });
                    onCloseAdd();
                  }}
                >
                  Agent · {agent}
                </button>
              ))
            )}
          </div>
        ) : null}
      </div>
    </div>
  );
}

function Board({
  items,
  selectedId,
  dropStatus,
  now,
  onSelect,
  onOpenAssign,
  onDropStatus,
  onDrop,
}: {
  items: readonly InboxItem[];
  selectedId: string | null;
  dropStatus: TriageStatus | null;
  now: number;
  onSelect: (id: string) => void;
  onOpenAssign: (item: InboxItem, node: HTMLElement) => void;
  onDropStatus: (status: TriageStatus | null) => void;
  onDrop: (status: TriageStatus, id: string) => void;
}) {
  return (
    <div className="inbox-columns" data-testid="inbox-board">
      {TRIAGE_STATUSES.map((status) => {
        const rows = items.filter((item) => item.status === status);
        return (
          <section
            key={status}
            className={dropStatus === status ? "column is-drop" : "column"}
            data-testid={`column-${status}`}
            onDragOver={(event) => {
              event.preventDefault();
              onDropStatus(status);
            }}
            onDragLeave={() => onDropStatus(null)}
            onDrop={(event) => {
              event.preventDefault();
              onDropStatus(null);
              onDrop(status, event.dataTransfer.getData("text/plain"));
            }}
          >
            <h2>
              {status}
              <span className="column-count">{rows.length}</span>
            </h2>
            {rows.length === 0 ? <p className="column-empty">Nothing here</p> : null}
            {rows.map((item) => (
              <Card
                key={item.verdict.id}
                item={item}
                selected={item.verdict.id === selectedId}
                now={now}
                onSelect={() => onSelect(item.verdict.id)}
                onOpenAssign={onOpenAssign}
              />
            ))}
          </section>
        );
      })}
    </div>
  );
}

function Card({
  item,
  selected,
  now,
  onSelect,
  onOpenAssign,
}: {
  item: InboxItem;
  selected: boolean;
  now: number;
  onSelect: () => void;
  onOpenAssign: (item: InboxItem, node: HTMLElement) => void;
}) {
  const chip = jevChip(item.verdict, useContext(WeightsContext));
  return (
    <div
      role="button"
      tabIndex={0}
      draggable
      className={selected ? "card is-selected" : "card"}
      data-testid={`inbox-item-${item.verdict.id}`}
      data-inbox-id={item.verdict.id}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (event.key === "Enter") onSelect();
      }}
      onDragStart={(event: DragEvent<HTMLDivElement>) => {
        event.dataTransfer.setData("text/plain", item.verdict.id);
        event.dataTransfer.effectAllowed = "move";
      }}
    >
      <span className="mono card-id" title={item.verdict.id}>
        {shortId(item.verdict.id)}
        {bindingLabel(item.verdict) ? ` · ${bindingLabel(item.verdict)}` : ""}
      </span>
      <span className="card-title">{cardTitle(item.verdict)}</span>
      <span className="card-subtitle">{cardSubtitle(item.verdict)}</span>
      <span className="card-facts">
        <span>{sourceLabel(item.verdict)}</span>
        <span>{amountLabel(item.verdict.amountCents)}</span>
        <span title="Age from the stored time. A dash means the stored time is not usable.">
          {ageLabel(item.verdict.recordedAt, now)}
        </span>
      </span>
      <span className="card-foot">
        <span className={`dot dot-${item.verdict.outcome}`} />
        <span className="card-evidence">{outcomeLabel(item.verdict.outcome)}</span>
        {chip ? <span className="jev-chip">{chip}</span> : null}
        <button
          type="button"
          className="avatar avatar-button"
          data-assignee={item.verdict.id}
          aria-label={`Assign ${item.verdict.id}`}
          onClick={(event) => {
            event.stopPropagation();
            onOpenAssign(item, event.currentTarget);
          }}
        >
          {initials(item.assignee)}
        </button>
      </span>
    </div>
  );
}

function InboxList({
  items,
  group,
  collapsed,
  selectedId,
  now,
  onToggle,
  onSelect,
  onOpenAssign,
}: {
  items: readonly InboxItem[];
  group: InboxGroupBy;
  collapsed: ReadonlySet<string>;
  selectedId: string | null;
  now: number;
  onToggle: (key: string) => void;
  onSelect: (id: string) => void;
  onOpenAssign: (item: InboxItem, node: HTMLElement) => void;
}) {
  const table = useTable({
    features,
    data: [...items],
    columns,
    getRowId: (row) => row.verdict.id,
  });
  const tableItems = table.getRowModel().rows.map((row) => row.original);
  const visibleGroups = groupItems(tableItems, group);
  const flat = flatten(visibleGroups, collapsed);
  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: flat.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 44,
    overscan: 8,
  });

  return (
    <div className="inbox-list" data-testid="inbox-list">
      <div className="list-head" role="row">
        <span />
        <span>Id</span>
        <span>Title</span>
        <span>Spec</span>
        <span>Source</span>
        <span>Amount</span>
        <span>Age</span>
        <span>Assignee</span>
      </div>
      <div className="list-scroll" ref={parentRef}>
      <div className="list-virtual" style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map((virtual) => {
          const row = flat[virtual.index];
          if (!row) return null;
          if (row.kind === "group") {
            const open = !collapsed.has(row.key);
            return (
              <div key={`group-${row.key}`} className="list-slot" style={{ transform: `translateY(${virtual.start}px)` }} ref={virtualizer.measureElement} data-index={virtual.index}>
                <button type="button" className="group-head" aria-expanded={open} onClick={() => onToggle(row.key)}>
                  <span>{open ? "▾" : "▸"}</span>
                  <span>{row.label}</span>
                  <span className="column-count">{row.count}</span>
                </button>
              </div>
            );
          }
          const item = row.item;
          return (
            <div key={item.verdict.id} className="list-slot" style={{ transform: `translateY(${virtual.start}px)` }} ref={virtualizer.measureElement} data-index={virtual.index}>
              <div
                role="button"
                tabIndex={0}
                className={item.verdict.id === selectedId ? "list-row is-selected" : "list-row"}
                data-testid={`inbox-item-${item.verdict.id}`}
                data-inbox-id={item.verdict.id}
                onClick={() => onSelect(item.verdict.id)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") onSelect(item.verdict.id);
                }}
              >
                <span className={`dot dot-${item.verdict.outcome}`} title={outcomeLabel(item.verdict.outcome)} />
                <span className="mono" title={item.verdict.id}>{shortId(item.verdict.id)}</span>
                <span className="row-title">{cardTitle(item.verdict)}</span>
                <span className="card-evidence">{cardSubtitle(item.verdict)}</span>
                <span className="card-evidence">{sourceLabel(item.verdict)}</span>
                <span className="card-evidence">{amountLabel(item.verdict.amountCents)}</span>
                <span className="card-evidence">{ageLabel(item.verdict.recordedAt, now)}</span>
                <button
                  type="button"
                  className="avatar avatar-button"
                  data-assignee={item.verdict.id}
                  aria-label={`Assign ${item.verdict.id}`}
                  onClick={(event) => {
                    event.stopPropagation();
                    onOpenAssign(item, event.currentTarget);
                  }}
                >
                  {initials(item.assignee)}
                </button>
              </div>
            </div>
          );
        })}
      </div>
      </div>
    </div>
  );
}

type FlatRow = { kind: "group"; key: string; label: string; count: number } | { kind: "item"; item: InboxItem };

function flatten(groups: readonly ItemGroup[], collapsed: ReadonlySet<string>): FlatRow[] {
  return groups.flatMap((group) => {
    const head: FlatRow = { kind: "group", key: group.key, label: group.label, count: group.items.length };
    if (collapsed.has(group.key)) return [head];
    return [head, ...group.items.map((item) => ({ kind: "item" as const, item }))];
  });
}

function Split({
  items,
  selected,
  now,
  onSelect,
  onOpenAssign,
  onReview,
  onDismiss,
}: {
  items: readonly InboxItem[];
  selected: InboxItem | null;
  now: number;
  onSelect: (id: string) => void;
  onOpenAssign: (item: InboxItem, node: HTMLElement) => void;
  onReview: () => void;
  onDismiss: () => void;
}) {
  const preview = selected ?? items[0] ?? null;
  return (
    <div className="split" data-testid="inbox-split">
      <ul className="feed">
        {items.map((item) => (
          <li key={item.verdict.id}>
            <button
              type="button"
              className={preview?.verdict.id === item.verdict.id ? "row is-selected" : "row"}
              data-testid={`inbox-item-${item.verdict.id}`}
              data-inbox-id={item.verdict.id}
              onClick={() => onSelect(item.verdict.id)}
            >
              <span className={`dot dot-${item.verdict.outcome}`} />
              <span className="mono">{shortId(item.verdict.id)}</span>
              <span className="row-title">{cardTitle(item.verdict)}</span>
              <span className="row-meta">{cardSubtitle(item.verdict)} · {item.status}</span>
            </button>
          </li>
        ))}
      </ul>
      {preview ? (
        <article className="split-detail" data-testid="split-detail">
          <p className="mono">{preview.verdict.id}</p>
          <h2 className="title-case">{cardTitle(preview.verdict)}</h2>
          <p className="card-subtitle">{cardSubtitle(preview.verdict)}</p>
          <p className={`outcome outcome-${preview.verdict.outcome}`}>{outcomeLabel(preview.verdict.outcome)}</p>
          <div className="inbox-actions">
            <button type="button" className="primary" onClick={onReview}>
              Mark reviewed
            </button>
            <button type="button" className="tool" onClick={onDismiss}>
              Dismiss
            </button>
            <button
              type="button"
              className="tool"
              data-assignee={preview.verdict.id}
              onClick={(event) => onOpenAssign(preview, event.currentTarget)}
            >
              <span className="avatar">{initials(preview.assignee)}</span>
              {preview.assignee ?? "Unassigned"}
            </button>
          </div>
          <dl className="props">
            <Prop label="Status" value={preview.status} />
            <Prop label="Spec" value={preview.verdict.specId || "—"} />
            <Prop label="Reason" value={preview.verdict.reasonCode || "—"} />
            <Prop label="Source" value={sourceLabel(preview.verdict)} />
            <Prop label="Amount" value={amountLabel(preview.verdict.amountCents)} />
            <Prop label="Age" value={ageLabel(preview.verdict.recordedAt, now)} />
            <Prop label="Binding" value={preview.verdict.binding || "No binding id was stored."} />
          </dl>
          <RankScoresView verdict={preview.verdict} />
          <h3 className="prop-label">Evidence</h3>
          {preview.verdict.evidence ? <p>{preview.verdict.evidence}</p> : <p className="reason">No evidence text on this verdict.</p>}
          <Timeline steps={preview.verdict.timeline ?? []} />
          <h3 className="prop-label">Binding comparison</h3>
          <dl className="props">
            <Prop label="Exact" value={bindingLine(preview.verdict.comparison?.exact)} />
            <Prop label="Fuzzy" value={bindingLine(preview.verdict.comparison?.fuzzy)} />
            <Prop label="Jev" value={bindingLine(preview.verdict.comparison?.jev)} />
          </dl>
        </article>
      ) : null}
    </div>
  );
}

function WeightSlider({
  label,
  testId,
  value,
  onChange,
}: {
  label: string;
  testId: string;
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="weight-row">
      <span>{label}</span>
      <span className="mono">{value}</span>
      <input
        type="range"
        min={0}
        max={100}
        step={1}
        aria-label={`${label} weight`}
        data-testid={testId}
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
      />
    </label>
  );
}

function RankScoresView({ verdict }: { verdict: InboxItem["verdict"] }) {
  const rank = verdict.rank;
  return (
    <div data-testid="rank-scores">
      <h3 className="prop-label">Priority scores</h3>
      <dl className="props">
        <Prop label="Money at stake" value={scoreLabel(rank, rank?.money)} />
        <Prop label="Evidence" value={scoreLabel(rank, rank?.evidence)} />
        <Prop label="Binding ambiguity" value={scoreLabel(rank, rank?.ambiguity)} />
        <Prop label="Repeated action" value={scoreLabel(rank, rank?.intent)} />
      </dl>
      <RankExchange rank={rank} />
    </div>
  );
}

function RankExchange({ rank }: { rank: InboxItem["verdict"]["rank"] }) {
  if (!rank) return <p className="reason">Scores are not stored for this verdict.</p>;
  const input = exchangeText(rank.input);
  const output = exchangeText(rank.output);
  if (!input && !output) return null;
  return (
    <details className="rank-exchange">
      <summary>Jev input and output</summary>
      {input ? <pre className="rank-json">{input}</pre> : null}
      {output ? <pre className="rank-json">{output}</pre> : null}
    </details>
  );
}

function exchangeText(value: unknown): string | null {
  if (value == null) return null;
  const text = JSON.stringify(value, null, 2);
  if (text.includes("TYPESAFE") || text.includes("Bearer") || text.includes("GEMINI")) return null;
  return text;
}

function Prop({ label, value }: { label: string; value: string }) {
  return (
    <div className="prop">
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}

function bindingLine(line: BindingLine | null | undefined): string {
  return line?.outcome ?? "Not stored";
}

function Timeline({ steps }: { steps: readonly { eventId: string; observedAt: string; source: string; kind: string; role: string }[] }) {
  if (steps.length === 0) {
    return <p className="reason">No evidence events on this verdict.</p>;
  }
  return (
    <ul className="timeline">
      {steps.map((step, index) => (
        <li key={`${step.eventId}-${index}`} className="timeline-event">
          <span className={`dot ${index === steps.length - 1 ? "dot-live" : "dot-span"}`} />
          <div className="timeline-copy">
            <div className="mono">{step.role || step.kind || "event"}</div>
            <div className="timeline-time">{step.observedAt || "—"}</div>
            <div className="timeline-evidence">{step.source}</div>
          </div>
        </li>
      ))}
    </ul>
  );
}
