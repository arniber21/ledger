import {
  columnFilteringFeature,
  createFilteredRowModel,
  tableFeatures,
  useTable,
  type ColumnDef,
} from "@tanstack/react-table";
import { Link, useNavigate } from "@tanstack/react-router";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useHotkeys, useToast } from "../ui";
import { amountLabel, scoreLabel } from "../inbox/model";
import { outcomeLabel } from "../stream/reduce";
import type { BindingComparison, BindingLine, VerdictItem } from "../stream/types";
import { clock } from "../traces/model";
import { useTraces } from "../traces/use-traces";
import { useShell } from "../shell/context";
import { Header } from "../shell/header";
import { VerdictsIcon } from "../shell/icons";
import {
  agentOf,
  assuranceOf,
  counterexampleView,
  entryDetail,
  entryLabel,
  eventFor,
  exchangeText as storedExchange,
  isViolating,
  relevanceLabel,
  sentryUrl,
  signatureLine,
  sourcesOf,
  tierFor,
  verdictTitle,
} from "./model";

const features = tableFeatures({
  columnFilteringFeature,
  filteredRowModel: createFilteredRowModel(),
});

type Features = typeof features;

const columns: ColumnDef<Features, VerdictItem>[] = [
  { accessorKey: "outcome", header: "Outcome" },
  { accessorKey: "title", header: "Title" },
  { accessorKey: "specId", header: "Spec" },
];

type DetailTab = "counterexample" | "evidence" | "binding";

export function VerdictsPage({ id }: { id?: string }) {
  const { metrics, reviewed, toggleReviewed, registerFeed, setSelectedId } = useShell();
  const traces = useTraces();
  const navigate = useNavigate();
  const verdicts = metrics.verdicts;
  const [tab, setTab] = useState<DetailTab>("counterexample");
  const [notes, setNotes] = useState<Record<string, string>>({});

  const table = useTable({
    features,
    data: verdicts,
    columns,
    getRowId: (row) => row.id,
    state: { columnFilters: [] },
    onColumnFiltersChange: () => undefined,
  });
  const rows = table.getRowModel().rows.map((row) => row.original);
  const feedKey = rows.map((row) => row.id).join("|");
  const selected = id ? rows.find((item) => item.id === id) : undefined;
  const toast = useToast();

  function review() {
    if (!selected || reviewed.has(selected.id)) return;
    const target = selected.id;
    toggleReviewed(target);
    toast.push({
      tone: "success",
      message: "Marked reviewed",
      undo: () => {
        if (reviewedRef.current.has(target)) toggleReviewed(target);
      },
    });
  }
  const reviewedRef = useRef(reviewed);
  reviewedRef.current = reviewed;

  useHotkeys(
    {
      e: { handler: review, description: "Mark reviewed" },
      "1": { handler: () => setTab("counterexample"), description: "Counterexample tab" },
      "2": { handler: () => setTab("evidence"), description: "Evidence tab" },
      "3": { handler: () => setTab("binding"), description: "Binding tab" },
      "mod+enter": { handler: review, description: "Mark reviewed (also inside the note)" },
      j: { passive: true, description: "Next verdict" },
      k: { passive: true, description: "Previous verdict" },
      escape: { passive: true, description: "Back" },
    },
    { scope: "Verdicts" },
  );

  useEffect(() => {
    if (!id) return;
    const node = document.querySelector(`[data-verdict-id="${CSS.escape(id)}"]`);
    if (!(node instanceof HTMLElement)) return;
    node.scrollIntoView({ block: "nearest" });
    const focused = document.activeElement;
    if (!focused || focused === document.body || focused.closest("[data-verdict-id]")) node.focus({ preventScroll: true });
  }, [id, feedKey]);

  useEffect(() => {
    registerFeed(feedKey.length === 0 ? [] : feedKey.split("|"));
    return () => registerFeed([]);
  }, [feedKey, registerFeed]);

  useEffect(() => {
    if (id) setSelectedId(id);
  }, [id, setSelectedId]);

  const open = (next: string) => {
    setSelectedId(next);
    void navigate({ to: "/verdicts", search: { id: next } });
  };

  const traceUrl = selected ? sentryUrl(traces.events, selected.timeline ?? []) : null;
  const meta = verdicts.length === 0 ? "No verdicts" : `${verdicts.length} on the stream`;

  return (
    <>
      <Header
        icon={<VerdictsIcon />}
        title="Verdicts"
        meta={meta}
        actions={selected ? <SentryAction url={traceUrl} /> : null}
      />
      <div className="content content-fill verdict-page">
        {verdicts.length === 0 ? (
          <Empty
            testId="verdicts-empty"
            retry={metrics.connection === "offline" ? () => window.location.reload() : undefined}
            title={id ? "Verdict is not on the stream" : "No verdicts yet"}
            body={
              id
                ? `${id} is not in the current snapshot.`
                : metrics.connection === "offline"
                  ? "The stream is offline. Satisfy, Violate, and Inconclusive outcomes show up here when it sends them. A missing signal stays inconclusive."
                  : "Satisfy, Violate, and Inconclusive outcomes show up here. A missing signal stays inconclusive."
            }
          />
        ) : (
          <div className="verdict-layout">
            <div className="verdict-feed" role="listbox" aria-label="Verdicts" aria-live="off">
              <p className="sr-only" role="status" aria-live="polite">{rows.length} {rows.length === 1 ? "verdict" : "verdicts"}</p>
              {rows.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  role="option"
                  aria-selected={item.id === id}
                  data-verdict-id={item.id}
                  className={item.id === id ? "card is-selected" : "card"}
                  data-testid={`verdict-row-${item.id}`}
                  data-outcome={item.outcome}
                  onClick={() => open(item.id)}
                >
                  <span className={`dot dot-${item.outcome}`} />
                  <span className="mono card-id">{item.id}</span>
                  <span className="card-title">{verdictTitle(item)}</span>
                  <span className={`badge badge-${item.outcome}`}>{outcomeLabel(item.outcome)}</span>
                  <span className="card-evidence">{item.specId || outcomeLabel(item.outcome)}</span>
                </button>
              ))}
            </div>
            {selected ? (
              <VerdictDetail
                verdict={selected}
                tab={tab}
                onTab={setTab}
                reviewed={reviewed.has(selected.id)}
                note={notes[selected.id] ?? ""}
                onNote={(value) => setNotes((current) => ({ ...current, [selected.id]: value }))}
                onReview={review}
                events={traces.events}
              />
            ) : (
              <Empty
                title={id ? "Verdict is not on the stream" : "Select a verdict"}
                body={
                  id
                    ? `${id} is not in the current snapshot.`
                    : "The timeline is the matched evidence for that verdict."
                }
              />
            )}
          </div>
        )}
      </div>
      <footer className="hintbar">
        <span>J and K move between verdicts · Enter opens · E marks reviewed · Z undoes · 1, 2, 3 switch tab</span>
        <span>A violate is needs review</span>
      </footer>
    </>
  );
}

function SentryAction({ url }: { url: string | null }) {
  if (!url) {
    return (
      <button type="button" className="tool" data-testid="verdict-sentry" disabled title="No Sentry trace on this verdict.">
        Open Sentry trace
      </button>
    );
  }
  return (
    <a className="tool" data-testid="verdict-sentry" href={url} target="_blank" rel="noreferrer">
      Open Sentry trace
    </a>
  );
}

function VerdictDetail({
  verdict,
  tab,
  onTab,
  reviewed,
  note,
  onNote,
  onReview,
  events,
}: {
  verdict: VerdictItem;
  tab: DetailTab;
  onTab: (tab: DetailTab) => void;
  reviewed: boolean;
  note: string;
  onNote: (value: string) => void;
  onReview: () => void;
  events: ReturnType<typeof useTraces>["events"];
}) {
  const timeline = verdict.timeline ?? [];
  const agent = agentOf(verdict);
  const sources = sourcesOf(verdict);
  const assurance = assuranceOf(events, timeline);

  return (
    <article className="spec-detail" data-testid="verdict-detail">
      <div className="trace-identity" data-testid="verdict-identity">
        <span className="spec-tile" aria-hidden="true">
          <span className={`dot dot-${verdict.outcome}`} />
        </span>
        <p className="mono trace-full-id">{verdict.id}</p>
        <h2 className="title-case">{verdictTitle(verdict)}</h2>
        <p className="trace-description">{verdict.evidence || "No evidence on this verdict."}</p>
        <dl className="props">
          <Prop label="Verdict">
            <span className={`dot dot-${verdict.outcome}`} />
            {outcomeLabel(verdict.outcome)}
          </Prop>
          <Prop label="Status">{reviewed ? "Reviewed" : "Open"}</Prop>
          <Prop label="Spec">
            {verdict.specId ? (
              <Link to="/specs" search={{ spec: verdict.specId }}>
                {verdict.specId}
              </Link>
            ) : (
              "Not on this verdict"
            )}
          </Prop>
          <Prop label="Agent">{agent ?? "Not on this verdict"}</Prop>
          <Prop label="Evidence">{sources ?? "Not on this verdict"}</Prop>
          <Prop label="Assurance">{assurance ?? "Not on this verdict"}</Prop>
          <Prop label="Binding">{verdict.binding || "Not on this verdict"}</Prop>
          <Prop label="Amount">{verdict.amountCents == null ? "Not on this verdict" : amountLabel(verdict.amountCents)}</Prop>
        </dl>
        <h3 className="prop-label">Priority scores</h3>
        <dl className="props" data-testid="rank-scores">
          <Prop label="Money at stake">{scoreLabel(verdict.rank, verdict.rank?.money)}</Prop>
          <Prop label="Evidence">{scoreLabel(verdict.rank, verdict.rank?.evidence)}</Prop>
          <Prop label="Binding ambiguity">{scoreLabel(verdict.rank, verdict.rank?.ambiguity)}</Prop>
          <Prop label="Repeated action">{scoreLabel(verdict.rank, verdict.rank?.intent)}</Prop>
        </dl>
        <RankExchange rank={verdict.rank} />
      </div>
      <div className="verdict-panel">
        <div className="spec-tabs" role="tablist" aria-label="Verdict detail">
          <Tab id="counterexample" label="Counterexample" current={tab} onSelect={onTab} />
          <Tab id="evidence" label="Evidence" current={tab} onSelect={onTab} />
          <Tab id="binding" label="Binding" current={tab} onSelect={onTab} />
        </div>
        {tab === "counterexample" ? <Counterexample key={verdict.id} verdict={verdict} events={events} /> : null}
        {tab === "evidence" ? <EvidenceList verdict={verdict} events={events} /> : null}
        <BindingCard comparison={verdict.comparison ?? null} />
      </div>
      <div className="review-note">
        <label htmlFor="verdict-note">Review note</label>
        <textarea
          id="verdict-note"
          data-testid="verdict-note"
          value={note}
          placeholder="Note for this review"
          onChange={(event) => onNote(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") event.currentTarget.blur();
            if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
              event.preventDefault();
              onReview();
            }
          }}
        />
        <button
          type="button"
          className="primary"
          data-testid="verdict-mark-reviewed"
          disabled={reviewed}
          onClick={onReview}
        >
          {reviewed ? "Reviewed" : "Mark reviewed"}
        </button>
      </div>
    </article>
  );
}

function Counterexample({
  verdict,
  events,
}: {
  verdict: VerdictItem;
  events: ReturnType<typeof useTraces>["events"];
}) {
  const [expanded, setExpanded] = useState(false);
  const [exchangeOpen, setExchangeOpen] = useState(false);
  const view = counterexampleView(verdict, expanded);
  const request = storedExchange(verdict.rerank?.request ?? "");
  const response = storedExchange(verdict.rerank?.response ?? "");
  if (view.steps.length === 0) {
    return (
      <p className="trace-description" role="tabpanel">
        No timeline on this verdict.
      </p>
    );
  }
  return (
    <div role="tabpanel">
      {view.unknown ? (
        <p className="trace-description" data-testid="timeline-unknown">
          Jev ranking is Unknown. The full trace is in time order.
        </p>
      ) : null}
      <ol className="timeline" aria-label="Counterexample">
        {view.steps.map((step, index) => {
          const violate = isViolating(verdict, step);
          const signature = signatureLine(events, step);
          const known = eventFor(events, step.eventId);
          return (
            <li
              key={`${step.eventId}-${index}`}
              className={violate ? "timeline-event is-violate" : "timeline-event"}
              data-testid="timeline-event"
            >
              <span className={violate ? "dot dot-Violate" : "dot dot-span"} />
              <div className="timeline-copy">
                <p className="mono">{entryLabel(step)}</p>
                <p className="timeline-time">{step.observedAt ? clock(step.observedAt) : "Time not on this event"}</p>
                <p className="timeline-evidence">{entryDetail(step)}</p>
                {signature ? <p className="signature">{signature}</p> : null}
                {known ? (
                  <Link to="/traces" search={{ trace: step.eventId }}>
                    Open trace
                  </Link>
                ) : null}
              </div>
            </li>
          );
        })}
      </ol>
      {view.hiddenCount > 0 ? (
        <button
          type="button"
          className="tool timeline-expand"
          data-testid="timeline-expand"
          aria-expanded={expanded}
          onClick={() => setExpanded((current) => !current)}
        >
          {expanded ? "Hide the rest" : `Show the rest (${view.hiddenCount})`}
        </button>
      ) : null}
      {verdict.rerank ? <RerankCard record={verdict.rerank} open={exchangeOpen} onToggle={() => setExchangeOpen((current) => !current)} request={request} response={response} /> : null}
    </div>
  );
}

function RerankCard({
  record,
  open,
  onToggle,
  request,
  response,
}: {
  record: NonNullable<VerdictItem["rerank"]>;
  open: boolean;
  onToggle: () => void;
  request: string;
  response: string;
}) {
  return (
    <div className="rerank" data-testid="verdict-rerank">
      {record.failure ? (
        <p className="trace-description">Jev ranking is Unknown ({record.failure}).</p>
      ) : null}
      {record.scores.length > 0 ? (
        <ul className="source-list">
          {record.scores.map((score) => (
            <li key={score.eventId} className="source-row" data-testid="rerank-score">
              <span className="mono">{score.eventId}</span>
              <span>{relevanceLabel(score.outcome)}</span>
            </li>
          ))}
        </ul>
      ) : null}
      {request || response ? (
        <button type="button" className="tool" data-testid="rerank-exchange" aria-pressed={open} onClick={onToggle}>
          {open ? "Hide Jev input" : "Show Jev input"}
        </button>
      ) : null}
      {open && request ? <pre className="rerank-body">{request}</pre> : null}
      {open && response ? <pre className="rerank-body">{response}</pre> : null}
    </div>
  );
}

function EvidenceList({
  verdict,
  events,
}: {
  verdict: VerdictItem;
  events: ReturnType<typeof useTraces>["events"];
}) {
  const timeline = verdict.timeline ?? [];
  return (
    <div role="tabpanel">
      <p className="trace-description">{verdict.evidence || "No evidence on this verdict."}</p>
      {verdict.reasonCode ? <p className="mono reason">{verdict.reasonCode}</p> : null}
      {timeline.length === 0 ? (
        <p className="trace-description">No evidence events on this verdict.</p>
      ) : (
        <ul className="source-list">
          {timeline.map((step, index) => (
            <li key={`${step.eventId}-${index}`} className="source-row">
              <span>{step.source || "Source not on this event"}</span>
              <span className="mono">{step.eventId}</span>
              <span>{eventFor(events, step.eventId) ? (tierFor(events, step.eventId) ?? "Not adapted") : "Not on this event"}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function BindingCard({ comparison }: { comparison: BindingComparison | null }) {
  const rows: { id: "exact" | "fuzzy" | "jev"; label: string; testId: string; line: BindingLine | null }[] = [
    { id: "exact", label: "Exact match", testId: "binding-exact", line: comparison?.exact ?? null },
    { id: "fuzzy", label: "Fuzzy matcher", testId: "binding-fuzzy", line: comparison?.fuzzy ?? null },
    { id: "jev", label: "Jev", testId: "binding-jev", line: comparison?.jev ?? null },
  ];
  return (
    <dl className="binding-card">
      {rows.map((row) => (
        <div key={row.id} className="binding-row" data-testid={row.testId}>
          <dt>{row.label}</dt>
          <dd>
            {row.line ? row.line.outcome : "Not on this verdict"}
            {row.line?.reasonCode ? <span className="binding-reason">{row.line.reasonCode}</span> : null}
          </dd>
        </div>
      ))}
    </dl>
  );
}

function Tab({
  id,
  label,
  current,
  onSelect,
}: {
  id: DetailTab;
  label: string;
  current: DetailTab;
  onSelect: (id: DetailTab) => void;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={current === id}
      data-testid={`verdict-tab-${id}`}
      onClick={() => onSelect(id)}
    >
      {label}
    </button>
  );
}

function RankExchange({ rank }: { rank: VerdictItem["rank"] }) {
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

function Prop({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="prop">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

function Empty({ title, body, testId, retry }: { title: string; body: string; testId?: string; retry?: () => void }) {
  return (
    <div className="empty" data-testid={testId}>
      <h2>{title}</h2>
      <p>{body}</p>
      {retry ? (
        <button type="button" className="tool" onClick={retry}>
          Retry
        </button>
      ) : null}
    </div>
  );
}
