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
import { useConfirm, useHotkeys, useToast } from "../ui";
import { useSetTriage, useTriageQuery, type TriagePatch } from "./use-triage";

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
  const triageQuery = useTriageQuery();
  const triage = useMemo(() => triageQuery.data ?? [], [triageQuery.data]);
  const setTriage = useSetTriage();
  const toast = useToast();
  const confirm = useConfirm();
  const [checked, setChecked] = useState<ReadonlySet<string>>(() => new Set());
  const [known, setKnown] = useState<ReadonlySet<string> | null>(null);
  const [fresh, setFresh] = useState<ReadonlySet<string>>(() => new Set());
  const [announce, setAnnounce] = useState("");
  const assignRef = useRef<HTMLInputElement>(null);
  const [assignIndex, setAssignIndex] = useState(0);
  const query = queryFromSearch(search);
  const [sliders, setSliders] = useState<RankSliders>(() => storedRankSliders());
  const [weightsOpen, setWeightsOpen] = useState(false);
  const weights = useMemo(() => weightsFromSliders(sliders), [sliders]);
  const full = useMemo(
    () => prepareInbox(metrics.verdicts, triage, query, weights),
    [metrics.verdicts, triage, query, weights],
  );
  // While a cursor is active, verdicts that arrive live wait behind an "N new" pill so the
  // list does not move under the cursor. With no cursor they join at once and flash.
  const frozen = known !== null && selectedId !== null;
  const prepared = useMemo(
    () =>
      frozen && known
        ? prepareInbox(metrics.verdicts.filter((verdict) => known.has(verdict.id)), triage, query, weights)
        : full,
    [frozen, known, metrics.verdicts, triage, query, weights, full],
  );
  const pendingNew = frozen && known ? full.items.filter((item) => !known.has(item.verdict.id)).length : 0;
  const fullIdKey = full.items.map((item) => item.verdict.id).join("|");
  const view = search.view ?? storedInboxView() ?? "board";
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set());
  const [adding, setAdding] = useState(false);
  const [assignOpen, setAssignOpen] = useState(false);
  const [assignQuery, setAssignQuery] = useState("");
  const [anchor, setAnchor] = useState({ top: 120, left: 280 });
  const [dropStatus, setDropStatus] = useState<TriageStatus | null>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  const addRef = useRef<HTMLDivElement>(null);
  const weightsRef = useRef<HTMLDivElement>(null);
  const now = Date.now();

  const items = prepared.items;
  const groups = prepared.groups;
  const active = items.find((item) => item.verdict.id === selectedId) ?? null;
  const calm = prepared.attention === 0 && !search.dismissed && !hasInboxFilters(search);
  const feedKey = orderedIds(view, items, groups, collapsed).join("|");

  useEffect(() => {
    if (metrics.verdicts.length === 0 && known === null) return;
    if (known === null) {
      setKnown(new Set(full.items.map((item) => item.verdict.id)));
      return;
    }
    if (!frozen) {
      const arrived = full.items.map((item) => item.verdict.id).filter((id) => !known.has(id));
      if (arrived.length === 0) return;
      setKnown(new Set(full.items.map((item) => item.verdict.id)));
      setFresh(new Set(arrived));
      const timer = window.setTimeout(() => setFresh(new Set()), 2600);
      return () => window.clearTimeout(timer);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fullIdKey, frozen]);

  function showNew() {
    if (!known) return;
    const arrived = full.items.map((item) => item.verdict.id).filter((id) => !known.has(id));
    setKnown(new Set(full.items.map((item) => item.verdict.id)));
    setFresh(new Set(arrived));
    window.setTimeout(() => setFresh(new Set()), 2600);
    setAnnounce(`${arrived.length} new ${arrived.length === 1 ? "verdict" : "verdicts"} added`);
  }

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
    if (!selectedId) return;
    const node = document.querySelector(`[data-inbox-id="${CSS.escape(selectedId)}"]`);
    if (!(node instanceof HTMLElement)) return;
    node.scrollIntoView({ block: "nearest" });
    const focused = document.activeElement;
    const free = !focused || focused === document.body || focused.closest("[data-inbox-id]") !== null;
    if (free) node.focus({ preventScroll: true });
  }, [selectedId, view, feedKey]);

  // Drop checked ids that left the list. The cursor itself survives refreshes.
  useEffect(() => {
    setChecked((current) => {
      if (current.size === 0) return current;
      const live = new Set(full.items.map((item) => item.verdict.id));
      const next = new Set([...current].filter((id) => live.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [fullIdKey]);

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

  function targets(): InboxItem[] {
    if (checked.size > 0) return items.filter((item) => checked.has(item.verdict.id));
    return active ? [active] : [];
  }

  function nextSelection(acted: readonly InboxItem[]): string | null {
    const ids = feedKey.length === 0 ? [] : feedKey.split("|");
    const gone = new Set(acted.map((item) => item.verdict.id));
    const anchorIndex = Math.max(...acted.map((item) => ids.indexOf(item.verdict.id)));
    const after = ids.slice(anchorIndex + 1).find((id) => !gone.has(id));
    return after ?? [...ids].reverse().find((id) => !gone.has(id)) ?? null;
  }

  async function apply(
    acted: readonly InboxItem[],
    change: (item: InboxItem) => { status: TriageStatus; assignee: string | null },
    message: (count: number) => string,
    advance: boolean,
  ) {
    if (acted.length === 0) return;
    const unsaved = acted.find((item) => !isUuid(item.verdict.id));
    if (unsaved) {
      toast.push({ tone: "error", message: "This verdict is not in the store, so its status was not saved." });
      return;
    }
    if (acted.length > 5) {
      const ok = await confirm({
        title: message(acted.length),
        body: `This changes ${acted.length} verdicts at once. You can undo it afterwards.`,
        confirmLabel: "Apply",
      });
      if (!ok) return;
    }
    const patches: TriagePatch[] = acted.map((item) => ({ verdictId: item.verdict.id, ...change(item) }));
    const before: TriagePatch[] = acted.map((item) => ({
      verdictId: item.verdict.id,
      status: item.status,
      assignee: item.assignee,
    }));
    const next = advance ? nextSelection(acted) : null;
    for (const patch of patches) setTriage.mutate(patch);
    setChecked(new Set());
    if (advance) setSelectedId(next);
    setAnnounce(message(acted.length));
    toast.push({
      tone: "success",
      message: message(acted.length),
      undo: () => {
        for (const patch of before) setTriage.mutate(patch);
        if (acted[0]) setSelectedId(acted[0].verdict.id);
      },
    });
  }

  function markReviewed() {
    void apply(targets(), (item) => ({ status: "Confirmed", assignee: item.assignee }), (n) => (n === 1 ? "Marked reviewed" : `Marked ${n} reviewed`), true);
  }

  function dismiss() {
    void apply(targets(), (item) => ({ status: "Dismissed", assignee: item.assignee }), (n) => (n === 1 ? "Dismissed" : `Dismissed ${n}`), true);
  }

  function assignTo(assignee: string | null) {
    const acted = targets();
    setAssignOpen(false);
    setAssignQuery("");
    void apply(
      acted,
      (item) => ({ status: item.status, assignee }),
      (n) => `${assignee ? "Assigned to " + assignee : "Unassigned"}${n === 1 ? "" : ` (${n})`}`,
      false,
    );
    refocusActive();
  }

  function refocusActive() {
    window.setTimeout(() => {
      if (selectedId) document.querySelector<HTMLElement>(`[data-inbox-id="${CSS.escape(selectedId)}"]`)?.focus({ preventScroll: true });
    }, 0);
  }

  function toggleChecked(id: string) {
    setChecked((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  function extend(delta: 1 | -1) {
    const ids = feedKey.length === 0 ? [] : feedKey.split("|");
    if (ids.length === 0) return;
    const index = selectedId ? ids.indexOf(selectedId) : -1;
    const target = ids[Math.min(ids.length - 1, Math.max(0, index === -1 ? 0 : index + delta))];
    if (!target) return;
    setChecked((current) => {
      const next = new Set(current);
      if (selectedId) next.add(selectedId);
      next.add(target);
      return next;
    });
    setSelectedId(target);
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

  const assignOptions: { id: string; label: string; value: string | null; key: string; avatar: string }[] = [
    { id: "none", label: "Unassigned", value: null, key: "0", avatar: "—" },
    { id: "me", label: "Me", value: "me", key: "1", avatar: "ME" },
  ].filter((option) => {
    const text = assignQuery.trim().toLowerCase();
    return text.length === 0 || option.label.toLowerCase().includes(text);
  });

  function onAssignKeyDown(event: React.KeyboardEvent<HTMLInputElement>) {
    event.stopPropagation();
    if (event.key === "Escape") {
      event.preventDefault();
      setAssignOpen(false);
      refocusActive();
    } else if (event.key === "ArrowDown" || (event.key === "n" && event.ctrlKey)) {
      event.preventDefault();
      setAssignIndex((i) => (assignOptions.length === 0 ? 0 : (i + 1) % assignOptions.length));
    } else if (event.key === "ArrowUp" || (event.key === "p" && event.ctrlKey)) {
      event.preventDefault();
      setAssignIndex((i) => (assignOptions.length === 0 ? 0 : (i - 1 + assignOptions.length) % assignOptions.length));
    } else if (event.key === "Enter") {
      event.preventDefault();
      const option = assignOptions[Math.min(assignIndex, assignOptions.length - 1)];
      if (option) assignTo(option.value);
    } else if (assignQuery === "" && (event.key === "0" || event.key === "1")) {
      event.preventDefault();
      assignTo(event.key === "0" ? null : "me");
    }
  }

  useEffect(() => {
    if (assignOpen) {
      setAssignIndex(0);
      assignRef.current?.focus();
    }
  }, [assignOpen]);

  useEffect(() => setAssignIndex(0), [assignQuery]);

  useEffect(() => {
    if (!assignOpen) return;
    const onPointer = (event: PointerEvent) => {
      if (!(event.target as HTMLElement).closest(".assign-menu")) setAssignOpen(false);
    };
    window.addEventListener("pointerdown", onPointer);
    return () => window.removeEventListener("pointerdown", onPointer);
  }, [assignOpen]);

  const noSelection = !active && checked.size === 0;
  useHotkeys(
    {
      e: { handler: markReviewed, description: "Mark reviewed (selection or checked)" },
      d: { handler: dismiss, description: "Dismiss" },
      a: { handler: () => (noSelection ? undefined : openAssign()), description: "Assign" },
      x: { handler: () => active && toggleChecked(active.verdict.id), description: "Check item for bulk action" },
      "shift+j": { handler: () => extend(1), description: "Extend checked range down" },
      "shift+k": { handler: () => extend(-1), description: "Extend checked range up" },
      "mod+a": { handler: () => setChecked(new Set(feedKey.length === 0 ? [] : feedKey.split("|"))), description: "Check everything" },
      ...(checked.size > 0
        ? {
            "!escape": {
              handler: () => {
                setChecked(new Set());
                setAnnounce("Selection cleared");
              },
              hidden: true,
            },
          }
        : {}),
      "1": { handler: () => selectView("board"), description: "Board view" },
      "2": { handler: () => selectView("list"), description: "List view" },
      "3": { handler: () => selectView("split"), description: "Split view" },
      "/": { handler: () => filterRef.current?.focus(), description: "Focus the filter" },
      j: { passive: true, description: "Move down" },
      k: { passive: true, description: "Move up" },
      enter: { passive: true, description: "Open the selected verdict" },
      escape: { passive: true, description: "Clear checks, then selection" },
    },
    { scope: "Inbox", enabled: !assignOpen },
  );

  const meta = calm ? "Nothing needs attention" : `${prepared.attention} need attention`;
  const chips = filterChips(search);
  
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
      <p className="sr-only" role="status" aria-live="polite" data-testid="inbox-live">
        {announce || `${items.length} ${items.length === 1 ? "verdict" : "verdicts"}${checked.size ? `, ${checked.size} checked` : ""}`}
      </p>
      {triageQuery.isError ? (
        <p className="notice" role="alert">
          Status is unavailable, so changes may not show.{" "}
          <button type="button" className="tool" onClick={() => void triageQuery.refetch()}>
            Retry
          </button>
        </p>
      ) : null}
      {pendingNew > 0 ? (
        <button type="button" className="new-pill" data-testid="new-pill" onClick={showNew}>
          {pendingNew} new
        </button>
      ) : null}
      {checked.size > 0 ? (
        <div className="bulk-bar" data-testid="bulk-bar" role="toolbar" aria-label="Bulk actions">
          <span>{checked.size} checked</span>
          <button type="button" className="tool" onClick={markReviewed}>Mark reviewed <kbd>E</kbd></button>
          <button type="button" className="tool" onClick={dismiss}>Dismiss <kbd>D</kbd></button>
          <button type="button" className="tool" onClick={openAssign}>Assign <kbd>A</kbd></button>
          <button type="button" className="tool" onClick={() => setChecked(new Set())}>Clear <kbd>Esc</kbd></button>
        </div>
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
                checked={checked}
                fresh={fresh}
                dropStatus={dropStatus}
                now={now}
                onSelect={setSelectedId}
                onOpenAssign={openAssignAt}
                onDropStatus={setDropStatus}
                onDrop={(status, id) => {
                  const item = items.find((entry) => entry.verdict.id === id);
                  if (!item || item.status === status) return;
                  void apply([item], () => ({ status, assignee: item.assignee }), () => `Moved to ${status}`, false);
                }}
              />
            ) : view === "list" ? (
              <InboxList
                items={items}
                group={query.group}
                collapsed={collapsed}
                selectedId={selectedId}
                checked={checked}
                fresh={fresh}
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
        <span>J and K move · X checks · Enter opens · E marks reviewed · D dismisses · A assigns · Z undoes</span>
        <span>1, 2, 3 switch view · / filters · ? all shortcuts</span>
      </footer>
      {assignOpen && (active || checked.size > 0) ? (
        <div
          className="assign-menu"
          role="dialog"
          aria-label="Assign"
          data-testid="assign-menu"
          style={{ top: anchor.top, left: anchor.left }}
        >
          <input
            ref={assignRef}
            className="inbox-query"
            value={assignQuery}
            placeholder="Search people"
            aria-label="Search people"
            role="combobox"
            aria-expanded="true"
            aria-controls="assign-options"
            aria-activedescendant={assignOptions[assignIndex] ? `assign-${assignOptions[assignIndex].id}` : undefined}
            onChange={(event) => setAssignQuery(event.target.value)}
            onKeyDown={onAssignKeyDown}
          />
          <div id="assign-options" role="listbox" aria-label="People">
            {assignOptions.map((option, index) => (
              <button
                key={option.id}
                id={`assign-${option.id}`}
                type="button"
                role="option"
                tabIndex={-1}
                aria-selected={index === assignIndex}
                data-active={index === assignIndex || undefined}
                onMouseEnter={() => setAssignIndex(index)}
                onClick={() => assignTo(option.value)}
              >
                <span className="avatar">{option.avatar}</span>
                <span>{option.label}</span>
                <kbd>{option.key}</kbd>
              </button>
            ))}
          </div>
          {assignOptions.length === 0 ? <p className="assign-empty">No one matches</p> : null}
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
  checked,
  fresh,
  dropStatus,
  now,
  onSelect,
  onOpenAssign,
  onDropStatus,
  onDrop,
}: {
  items: readonly InboxItem[];
  selectedId: string | null;
  checked: ReadonlySet<string>;
  fresh: ReadonlySet<string>;
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
            <div role="listbox" aria-multiselectable="true" aria-label={`${status}, ${rows.length}`} className="column-list">
            {rows.map((item) => (
              <Card
                key={item.verdict.id}
                item={item}
                selected={item.verdict.id === selectedId}
                checked={checked.has(item.verdict.id)}
                fresh={fresh.has(item.verdict.id)}
                now={now}
                onSelect={() => onSelect(item.verdict.id)}
                onOpenAssign={onOpenAssign}
              />
            ))}
            </div>
          </section>
        );
      })}
    </div>
  );
}

function Card({
  item,
  selected,
  checked,
  fresh,
  now,
  onSelect,
  onOpenAssign,
}: {
  item: InboxItem;
  selected: boolean;
  checked: boolean;
  fresh: boolean;
  now: number;
  onSelect: () => void;
  onOpenAssign: (item: InboxItem, node: HTMLElement) => void;
}) {
  const chip = jevChip(item.verdict, useContext(WeightsContext));
  return (
    <div
      role="option"
      aria-selected={selected || checked}
      aria-label={`${cardTitle(item.verdict)}, ${item.status}`}
      tabIndex={selected ? 0 : -1}
      draggable
      className={`card${selected ? " is-selected" : ""}${checked ? " is-checked" : ""}${fresh ? " is-fresh" : ""}`}
      data-testid={`inbox-item-${item.verdict.id}`}
      data-inbox-id={item.verdict.id}
      onClick={onSelect}
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
  checked,
  fresh,
  now,
  onToggle,
  onSelect,
  onOpenAssign,
}: {
  items: readonly InboxItem[];
  group: InboxGroupBy;
  collapsed: ReadonlySet<string>;
  selectedId: string | null;
  checked: ReadonlySet<string>;
  fresh: ReadonlySet<string>;
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
  useEffect(() => {
    if (!selectedId) return;
    const index = flat.findIndex((row) => row.kind === "item" && row.item.verdict.id === selectedId);
    if (index >= 0) virtualizer.scrollToIndex(index, { align: "auto" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId]);

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
      <div className="list-virtual" role="listbox" aria-multiselectable="true" aria-label="Verdicts" style={{ height: virtualizer.getTotalSize() }}>
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
                role="option"
                aria-selected={item.verdict.id === selectedId || checked.has(item.verdict.id)}
                tabIndex={item.verdict.id === selectedId ? 0 : -1}
                className={`list-row${item.verdict.id === selectedId ? " is-selected" : ""}${checked.has(item.verdict.id) ? " is-checked" : ""}${fresh.has(item.verdict.id) ? " is-fresh" : ""}`}
                data-testid={`inbox-item-${item.verdict.id}`}
                data-inbox-id={item.verdict.id}
                onClick={() => onSelect(item.verdict.id)}
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
      <ul className="feed" role="listbox" aria-label="Verdicts">
        {items.map((item) => (
          <li key={item.verdict.id}>
            <button
              type="button"
              role="option"
              aria-selected={preview?.verdict.id === item.verdict.id}
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
