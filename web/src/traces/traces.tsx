import { useNavigate } from "@tanstack/react-router";
import {
  columnFilteringFeature,
  createFilteredRowModel,
  tableFeatures,
  useTable,
  type ColumnDef,
} from "@tanstack/react-table";
import { useEffect, useMemo, useState } from "react";
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

const FAULTS = ["double-pay", "no-approval", "exceed-budget", "swapped-vendor", "hidden-memo"] as const;

type FaultName = (typeof FAULTS)[number];

type DetailTab = "spans" | "sources" | "attributes";

export function TracesPage({ traceId }: { traceId?: string }) {
  const snapshot = useTraces();
  const { registerFeed, selectedId, setSelectedId, reportFaultUnavailable } = useShell();
  const navigate = useNavigate();
  const [enabled, setEnabled] = useState<Partial<Record<FaultName, boolean>>>({});
  const [pending, setPending] = useState<FaultName | null>(null);
  const [tab, setTab] = useState<DetailTab>("spans");

  const events = snapshot.events;
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

  const toggleFault = async (name: FaultName) => {
    setPending(name);
    try {
      const response = await fetch(`/api/faults/${name}`, { method: "POST" });
      if (!response.ok) {
        reportFaultUnavailable();
        return;
      }
      const body: unknown = await response.json();
      if (typeof body === "object" && body !== null && "enabled" in body && typeof body.enabled === "boolean") {
        const isEnabled = body.enabled;
        setEnabled((current) => ({ ...current, [name]: isEnabled }));
      }
    } catch {
      reportFaultUnavailable();
    } finally {
      setPending(null);
    }
  };

  return (
    <>
      <Header icon={<TracesIcon />} title="Traces" meta={meta} />
      <div className="content content-fill trace-page">
        <div className="faults" role="group" aria-label="Faults">
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
              onClick={() => void toggleFault(name)}
            >
              {name}
            </button>
          ))}
        </div>
        {events.length === 0 ? (
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
            <ul className="feed trace-feed" data-testid="trace-feed">
              {rows.map((event) => {
                const root = traceIds.get(event.id) ?? event.id;
                const selected = trace?.id === root;
                return (
                  <li key={`${event.id}|${event.observedAt}`}>
                    <button
                      type="button"
                      className={selected ? "row trace-row is-selected" : "row trace-row"}
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
        <span>J and K move between spans · Enter opens the trace</span>
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
