import { useNavigate } from "@tanstack/react-router";
import {
  columnFilteringFeature,
  createFilteredRowModel,
  tableFeatures,
  useTable,
  type ColumnDef,
} from "@tanstack/react-table";
import { useEffect, useMemo, useRef, useState } from "react";
import { useHotkeys, useToast } from "../ui";
import { FAULTS, useFaultControl, type FaultName } from "../shell/faults";
import { Header } from "../shell/header";
import { TracesIcon } from "../shell/icons";
import { useShell } from "../shell/context";
import {
  agentOf,
  clock,
  companyOf,
  evidenceSentence,
  groupTraces,
  initials,
  settleLines,
  signatureOf,
  sourcesOf,
  spanName,
  tiersOf,
  traceDescription,
  traceDuration,
  traceFor,
  traceTitle,
  type StoredEvent,
  type Trace,
} from "./model";
import { useTraces } from "./use-traces";

const features = tableFeatures({
  columnFilteringFeature,
  filteredRowModel: createFilteredRowModel(),
});

type Features = typeof features;

type DetailTab = "spans" | "sources" | "attributes";

export function TracesPage({ traceId }: { traceId?: string }) {
  const snapshot = useTraces();
  const { registerFeed, selectedId, setSelectedId } = useShell();
  const toast = useToast();
  const navigate = useNavigate();
  const [enabled, setEnabled] = useState<Partial<Record<FaultName, boolean>>>({});
  const [tab, setTab] = useState<DetailTab>("spans");

  const live = snapshot.events;
  const [paused, setPaused] = useState(false);
  const [scrolledAway, setScrolledAway] = useState(false);
  const frozen = useRef<Set<string> | null>(null);
  const feedRef = useRef<HTMLUListElement>(null);
  const [sawLive, setSawLive] = useState(false);
  const [waited, setWaited] = useState(false);
  const seen = useRef<Set<string> | null>(null);
  const [fresh, setFresh] = useState<ReadonlySet<string>>(() => new Set());

  // Hold the list still while the cursor is on it. New spans wait behind the pill.
  const holding = paused || scrolledAway || (selectedId !== null && live.some((event) => event.id === selectedId));
  if (holding && frozen.current === null) frozen.current = new Set(live.map((event) => event.id));
  if (!holding && frozen.current !== null) frozen.current = null;
  const events = holding && frozen.current ? live.filter((event) => frozen.current?.has(event.id)) : live;
  const hiddenCount = live.length - events.length;

  useEffect(() => {
    if (snapshot.connection === "live") setSawLive(true);
  }, [snapshot.connection]);
  useEffect(() => {
    const timer = window.setTimeout(() => setWaited(true), 3000);
    return () => window.clearTimeout(timer);
  }, []);

  // Spans that arrive after first paint get a short highlight.
  useEffect(() => {
    if (seen.current === null) {
      seen.current = new Set(live.map((event) => event.id));
      return;
    }
    const added = live.filter((event) => !seen.current?.has(event.id)).map((event) => event.id);
    if (added.length === 0) return;
    for (const id of added) seen.current.add(id);
    setFresh((current) => new Set([...current, ...added]));
    const timer = window.setTimeout(
      () => setFresh((current) => new Set([...current].filter((id) => !added.includes(id)))),
      2400,
    );
    return () => window.clearTimeout(timer);
  }, [live]);

  const release = () => {
    setPaused(false);
    setScrolledAway(false);
    setSelectedId(null);
    frozen.current = null;
    const list = feedRef.current;
    if (list) list.scrollTop = list.scrollHeight;
  };
  const traces = useMemo(() => groupTraces(events), [events]);
  const traceIds = useMemo(() => {
    const ids = new Map<string, string>();
    for (const trace of traces) {
      for (const span of trace.spans) ids.set(span.id, trace.id);
    }
    return ids;
  }, [traces]);

  const columns = useMemo<ColumnDef<Features, StoredEvent>[]>(
    () => [
      { accessorKey: "source", header: "Source" },
      { id: "span", accessorFn: (row) => spanName(row), header: "Span" },
      { id: "evidence", accessorFn: (row) => evidenceSentence(row), header: "Evidence" },
      { id: "trace", accessorFn: (row) => traceIds.get(row.id) ?? row.id, header: "Trace" },
    ],
    [traceIds],
  );

  const table = useTable({
    features,
    data: events,
    columns,
    getRowId: (row) => row.id,
  });
  const rows = table.getRowModel().rows.map((row) => row.original);
  const feedKey = rows.map((row) => row.id).join("|");

  useEffect(() => {
    registerFeed(feedKey.length === 0 ? [] : feedKey.split("|"));
    return () => registerFeed([]);
  }, [feedKey, registerFeed]);

  const activeId = events.some((event) => event.id === selectedId) ? selectedId : traceId;
  const trace = traceFor(traces, activeId ?? undefined);

  const meta =
    events.length === 0
      ? snapshot.connection === "live"
        ? "Ingest live"
        : "No traces yet"
      : `${events.length} ${events.length === 1 ? "span" : "spans"}`;

  const open = (event: StoredEvent) => {
    setSelectedId(event.id);
    void navigate({ to: "/traces", search: { trace: event.id } });
  };

  const fault = useFaultControl((name, isEnabled) => setEnabled((current) => ({ ...current, [name]: isEnabled })));
  const pending = fault.pending;
  const toggleFault = (name: FaultName) => void fault.toggle(name);
  const faultsRef = useRef<HTMLDivElement>(null);

  // Keep the selected row visible. The rows are real buttons, so focus follows the cursor.
  useEffect(() => {
    if (!selectedId) return;
    const row = feedRef.current?.querySelector<HTMLElement>(`[data-span-id="${CSS.escape(selectedId)}"]`);
    if (!row) return;
    row.scrollIntoView({ block: "nearest" });
    const active = document.activeElement;
    if (!(active instanceof HTMLElement) || active === document.body || feedRef.current?.contains(active)) row.focus({ preventScroll: true });
  }, [selectedId]);

  const copySpan = async () => {
    const id = selectedId ?? events[events.length - 1]?.id;
    if (!id) return;
    try {
      await navigator.clipboard.writeText(id);
      toast.push({ message: "Copied", tone: "success", durationMs: 1800 });
    } catch {
      toast.push({ message: "Could not copy. Select the id and copy it by hand.", tone: "error" });
    }
  };

  useHotkeys(
    {
      f: { handler: () => faultsRef.current?.querySelector<HTMLElement>("button")?.focus(), description: "Open the fault menu" },
      p: { handler: () => setPaused((current) => !current), description: "Pause or resume the live feed" },
      c: { handler: () => void copySpan(), description: "Copy the span id" },
      "!escape": {
        handler: (event) => {
          if (!(event.target instanceof HTMLElement) || !faultsRef.current?.contains(event.target)) return;
          event.stopPropagation();
          feedRef.current?.querySelector<HTMLElement>("button.is-selected, button")?.focus();
        },
        hidden: true,
      },
    },
    { scope: "Traces" },
  );

  return (
    <>
      <Header
        icon={<TracesIcon />}
        title="Traces"
        meta={paused ? `${meta} · paused` : meta}
        actions={
          <button
            type="button"
            className="tool"
            data-testid="trace-pause"
            aria-pressed={paused}
            onClick={() => setPaused((current) => !current)}
          >
            {paused ? "Resume" : "Pause"} <kbd>P</kbd>
          </button>
        }
      />
      <div className="content content-fill trace-page">
        <div
          className="faults"
          role="group"
          aria-label="Faults"
          ref={faultsRef}
          onKeyDown={(event) => {
            if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
            const buttons = Array.from(faultsRef.current?.querySelectorAll<HTMLElement>("button") ?? []);
            const at = buttons.indexOf(document.activeElement as HTMLElement);
            if (at === -1) return;
            event.preventDefault();
            buttons[(at + (event.key === "ArrowRight" ? 1 : -1) + buttons.length) % buttons.length]?.focus();
          }}
        >
          <span>Faults</span>
          {FAULTS.map((name) => (
            <button
              key={name}
              type="button"
              className="fault"
              data-testid={`fault-${name}`}
              aria-pressed={enabled[name] === true}
              aria-busy={pending === name}
              disabled={pending !== null}
              onClick={() => toggleFault(name)}
            >
              {name}
            </button>
          ))}
        </div>
        {events.length === 0 && !sawLive && !waited ? (
          <div className="skeleton-list" data-testid="trace-loading" role="status" aria-label="Loading spans">
            {[0, 1, 2, 3, 4].map((index) => (
              <div key={index} className="skeleton-row" />
            ))}
          </div>
        ) : events.length === 0 && !sawLive ? (
          <div className="empty" data-testid="trace-error" role="alert">
            <h2>Ingest is not reachable</h2>
            <p>The trace stream has not connected. It retries on its own. Nothing is shown until spans arrive.</p>
          </div>
        ) : events.length === 0 ? (
          <div className="empty" data-testid="trace-empty">
            <h2>{traceId ? "Trace is not on the stream" : "No traces yet"}</h2>
            <p>
              {traceId
                ? `${traceId} has not arrived. Spans show up here when ingest is connected.`
                : "Spans from the agents and the exchange show up here when ingest is connected."}
            </p>
          </div>
        ) : (
          <div className="trace-layout">
            <div className="trace-feed-wrap">
            {hiddenCount > 0 ? (
              <button type="button" className="new-pill" data-testid="new-spans" onClick={release}>
                {hiddenCount} new {hiddenCount === 1 ? "span" : "spans"}
              </button>
            ) : null}
            <ul
              className="feed trace-feed"
              data-testid="trace-feed"
              ref={feedRef}
              aria-label="Spans"
              onScroll={(event) => {
                const list = event.currentTarget;
                setScrolledAway(list.scrollHeight - list.scrollTop - list.clientHeight > 48);
              }}
            >
              {rows.map((event) => {
                const root = traceIds.get(event.id) ?? event.id;
                const selected = trace?.id === root;
                return (
                  <li key={`${event.id}|${event.observedAt}`}>
                    <button
                      type="button"
                      data-span-id={event.id}
                      aria-current={event.id === selectedId ? "true" : undefined}
                      className={`row trace-row${selected ? " is-selected" : ""}${fresh.has(event.id) ? " is-fresh" : ""}`}
                      onClick={() => open(event)}
                    >
                      <span className="trace-source">{event.source}</span>
                      <span className="mono trace-span">{spanName(event)}</span>
                      <span className="trace-evidence">{evidenceSentence(event)}</span>
                      <span className="mono trace-id" title={root}>
                        {root.slice(0, 8)}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
            </div>
            {trace ? (
              <TraceDetail trace={trace} tab={tab} onTab={setTab} liveId={trace.spans[trace.spans.length - 1]?.id} />
            ) : (
              <div className="empty">
                <h2>{traceId ? "Trace is not on the stream" : "Select a trace"}</h2>
                <p>
                  {traceId
                    ? `${traceId} has not arrived. Spans show up here when ingest is connected.`
                    : "Choose a span from the feed. New spans append as ingest stores them."}
                </p>
              </div>
            )}
          </div>
        )}
      </div>
      <footer className="hintbar">
        <span>J and K move · Enter opens · F faults · P pause · C copy span id · Esc back to list</span>
        <span>A fault toggles the running desk</span>
      </footer>
    </>
  );
}

function TraceDetail({
  trace,
  tab,
  onTab,
  liveId,
}: {
  trace: Trace;
  tab: DetailTab;
  onTab: (tab: DetailTab) => void;
  liveId: string | undefined;
}) {
  const agent = agentOf(trace.spans);
  const duration = traceDuration(trace.spans);
  const signature = signatureOf(trace.spans);

  return (
    <article className="trace-detail" data-testid="trace-detail">
      <div className="trace-identity">
        <span className="trace-tile">
          <TracesIcon />
        </span>
        <p className="mono trace-full-id">{trace.id}</p>
        <h2>{traceTitle(trace)}</h2>
        <p className="trace-description">{traceDescription(trace)}</p>
        <dl className="props">
          <div className="prop">
            <dt>Agent</dt>
            <dd>
              <span className="avatar" aria-hidden="true">
                {initials(agent)}
              </span>
              {agent}
            </dd>
          </div>
          <div className="prop">
            <dt>Company</dt>
            <dd>{companyOf(trace.spans)}</dd>
          </div>
          <div className="prop">
            <dt>Spans</dt>
            <dd>{trace.spans.length}</dd>
          </div>
          <div className="prop">
            <dt>Duration</dt>
            <dd>{duration ?? "Unknown"}</dd>
          </div>
          <div className="prop">
            <dt>Sources</dt>
            <dd>{sourcesOf(trace.spans)}</dd>
          </div>
          <div className="prop">
            <dt>Sampling</dt>
            <dd>Not sampled</dd>
          </div>
          <div className="prop">
            <dt>Tier</dt>
            <dd>{tiersOf(trace.spans)}</dd>
          </div>
        </dl>
        <p className="prop-label">Transaction</p>
        <p className="signature" data-testid="trace-signature">
          {signature ?? "No transaction signature on this trace."}
        </p>
      </div>
      <div className="trace-panel">
        <div className="views" role="group" aria-label="Trace detail">
          <button type="button" data-testid="tab-spans" aria-pressed={tab === "spans"} onClick={() => onTab("spans")}>
            Spans
          </button>
          <button
            type="button"
            data-testid="tab-sources"
            aria-pressed={tab === "sources"}
            onClick={() => onTab("sources")}
          >
            Sources
          </button>
          <button
            type="button"
            data-testid="tab-attributes"
            aria-pressed={tab === "attributes"}
            onClick={() => onTab("attributes")}
          >
            Attributes
          </button>
        </div>
        {tab === "spans" ? <SpanTimeline trace={trace} liveId={liveId} /> : null}
        {tab === "sources" ? <SourceList trace={trace} /> : null}
        {tab === "attributes" ? <AttributeList trace={trace} /> : null}
      </div>
    </article>
  );
}

function SpanTimeline({ trace, liveId }: { trace: Trace; liveId: string | undefined }) {
  return (
    <ol className="timeline">
      {trace.spans.map((span) => {
        const lines = settleLines(span);
        const live = span.id === liveId;
        return (
          <li key={span.id} className="timeline-event" data-testid="timeline-event">
            <span className={live ? "dot dot-live" : "dot dot-span"} />
            <div className="timeline-copy">
              <p className="mono">{spanName(span)}</p>
              <p className="timeline-time">{clock(span.payload.closedAt ?? span.observedAt)}</p>
              <p className="timeline-evidence">{evidenceSentence(span)}</p>
              {lines ? (
                <div className="attr-card">
                  {lines.map((line, index) => (
                    <p key={line} className="attr-line">
                      <span>{index + 1}</span>
                      <span>{line}</span>
                    </p>
                  ))}
                </div>
              ) : null}
            </div>
          </li>
        );
      })}
    </ol>
  );
}

function SourceList({ trace }: { trace: Trace }) {
  return (
    <ul className="source-list">
      {trace.spans.map((span) => (
        <li key={span.id} className="source-row">
          <span>{span.source}</span>
          <span className="mono">{spanName(span)}</span>
          <span>{span.payload.sentinel?.tier ?? "Not adapted"}</span>
        </li>
      ))}
    </ul>
  );
}

function AttributeList({ trace }: { trace: Trace }) {
  const lines = trace.spans.flatMap((span) => {
    const entries = Object.entries(span.payload.attributes).map(([key, value]) => `${spanName(span)}  ${key}  ${value}`);
    const tier = span.payload.sentinel?.tier;
    return tier ? [...entries, `${spanName(span)}  sentinel.tier  ${tier}`] : entries;
  });
  if (lines.length === 0) {
    return <p className="trace-description">No attributes on this trace.</p>;
  }
  return (
    <div className="attr-card">
      {lines.map((line, index) => (
        <p key={`${index}-${line}`} className="attr-line">
          <span>{index + 1}</span>
          <span>{line}</span>
        </p>
      ))}
    </div>
  );
}
