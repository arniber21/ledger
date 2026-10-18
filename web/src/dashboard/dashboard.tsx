import {
  columnFilteringFeature,
  createFilteredRowModel,
  tableFeatures,
  useTable,
  type ColumnDef,
  type ColumnFiltersState,
} from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useMemo, useRef } from "react";
import { isOutcome, outcomeLabel } from "../stream/reduce";
import type { SavedView, VerdictItem, VerdictOutcome } from "../stream/types";
import { Calibration } from "./calibration-panel";
import { AttentionChart, EventsChart, MonitorsChart } from "./charts";
import { ageLabel, amountLabel, bindingLabel, cardSubtitle, cardTitle, shortId, sourceLabel } from "../inbox/model";
import { useShell } from "../shell/context";
import { DashboardIcon } from "../shell/icons";
import { Header } from "../shell/header";

const features = tableFeatures({
  columnFilteringFeature,
  filteredRowModel: createFilteredRowModel(),
});

type Features = typeof features;

const columns: ColumnDef<Features, VerdictItem>[] = [
  {
    accessorKey: "outcome",
    header: "Outcome",
    filterFn: (row, columnId, value) => row.getValue(columnId) === value,
  },
  {
    accessorKey: "assignedToMe",
    header: "Assigned",
    filterFn: (row, columnId, value) => row.getValue(columnId) === value,
  },
  {
    accessorKey: "jevUnknown",
    header: "Jev unknown",
    filterFn: (row, columnId, value) => row.getValue(columnId) === value,
  },
  {
    accessorKey: "amountCents",
    header: "Amount",
    filterFn: (row, _columnId, value) => {
      const amount = row.original.amountCents;
      return typeof value === "number" && amount != null && amount >= value;
    },
  },
  { accessorKey: "title", header: "Title" },
  { accessorKey: "evidence", header: "Evidence" },
];

const countFormat = new Intl.NumberFormat("en-US");
const rateFormat = new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 });

function filtersFor(view: SavedView | undefined, outcome: VerdictOutcome | undefined): ColumnFiltersState {
  if (outcome) return [{ id: "outcome", value: outcome }];
  switch (view) {
    case "assigned":
      return [{ id: "assignedToMe", value: true }];
    case "violations":
      return [{ id: "outcome", value: "Violate" }];
    case "unknowns":
      return [{ id: "jevUnknown", value: true }];
    case "over-1000":
      return [{ id: "amountCents", value: 100_000 }];
    default:
      return [];
  }
}

export function Dashboard({
  view,
  outcome,
}: {
  view?: SavedView;
  outcome?: VerdictOutcome;
}) {
  const { metrics, registerFeed, selectedId, setSelectedId, reviewed, openPalette } = useShell();
  const navigate = useNavigate();
  const columnFilters = filtersFor(view, outcome);

  const table = useTable({
    features,
    data: metrics.verdicts,
    columns,
    getRowId: (row) => row.id,
    state: { columnFilters },
    onColumnFiltersChange: () => undefined,
  });

  const rows = table.getRowModel().rows.map((row) => row.original);
  const needsReview = rows.filter((row) => row.outcome === "Violate");
  const inconclusive = rows.filter((row) => row.outcome === "Inconclusive");
  const satisfied = rows.filter((row) => row.outcome === "Satisfy");
  const feedKey = rows.map((row) => row.id).join("|");

  useEffect(() => {
    const ids = feedKey.length === 0 ? [] : feedKey.split("|");
    registerFeed(ids);
    return () => registerFeed([]);
  }, [feedKey, registerFeed]);

  const open = (id: string) => {
    setSelectedId(id);
    void navigate({ to: "/verdicts", search: { id } });
  };

  const chip = chipLabel(view, outcome);

  return (
    <>
      <Header icon={<DashboardIcon />} title="Dashboard" />
      <div className="content">
        <section className="metrics" aria-label="Metrics">
          <Metric
            testId="metric-events"
            label="Events ingested"
            value={metrics.eventsIngested}
            caption={`${rateFormat.format(metrics.eventsPerSecond)} per second, ${metrics.sourceCount} ${
              metrics.sourceCount === 1 ? "source" : "sources"
            }`}
          />
          <Metric
            testId="metric-monitors"
            label="Live monitors"
            value={metrics.liveMonitors}
            caption={monitorCaption(metrics.liveMonitors, metrics.monitors)}
          />
          <Metric
            testId="metric-retired"
            label="Retired by GC"
            value={metrics.retiredByGc}
            caption={`Jev closed ${countFormat.format(metrics.jevClosedAmbiguous)} ambiguous`}
          />
          <Metric
            testId="metric-settled"
            label="Settled on Solana"
            value={metrics.settledOnSolana}
            caption={`${countFormat.format(metrics.settlementsFailed)} failed`}
          />
          <Metric
            testId="metric-jev"
            label="Jev judgments"
            value={metrics.jevJudgments}
            caption={`${countFormat.format(metrics.jevUnknown)} returned Unknown`}
          />
          <button type="button" className="inject" data-testid="action-inject-fault" onClick={openPalette}>
            Inject a fault
          </button>
        </section>

        <section className="charts" aria-label="Charts">
          <EventsChart points={metrics.rates} />
          <AttentionChart points={metrics.attention} />
          <MonitorsChart points={metrics.monitors} />
        </section>

        <Calibration />

        {chip ? (
          <div className="chips">
            <button
              type="button"
              className="chip"
              onClick={() => {
                void navigate({ to: "/", search: { view: undefined, outcome: undefined } });
              }}
            >
              {chip}
              <span aria-hidden="true">×</span>
            </button>
          </div>
        ) : null}

        <section className="board" aria-label="Verdicts">
          <Column
            title="Needs review"
            tone="Violate"
            rows={needsReview}
            selectedId={selectedId}
            reviewed={reviewed}
            onOpen={open}
          />
          <Column
            title="Inconclusive"
            tone="Inconclusive"
            rows={inconclusive}
            selectedId={selectedId}
            reviewed={reviewed}
            onOpen={open}
          />
          <Column
            title="Satisfied"
            tone="Satisfy"
            rows={satisfied}
            selectedId={selectedId}
            reviewed={reviewed}
            onOpen={open}
          />
        </section>
      </div>
    </>
  );
}

function chipLabel(view: SavedView | undefined, outcome: VerdictOutcome | undefined): string | null {
  if (outcome) return outcomeLabel(outcome);
  switch (view) {
    case "assigned":
      return "Assigned to me";
    case "violations":
      return "Violations only";
    case "unknowns":
      return "Jev unknowns";
    case "over-1000":
      return "Over $1,000";
    default:
      return null;
  }
}

function monitorCaption(
  live: number,
  points: { withGc: number }[],
): string {
  if (points.length >= 2 && points[0]?.withGc === points.at(-1)?.withGc) return "Flat over the window";
  if (points.length > 0) return "Across the last 10 minutes";
  if (live === 0) return "None live";
  return "From the stream";
}

function Metric({
  testId,
  label,
  value,
  caption,
}: {
  testId: string;
  label: string;
  value: number;
  caption: string;
}) {
  return (
    <article className="metric" data-testid={testId}>
      <h2>{label}</h2>
      <p className="metric-value">{countFormat.format(value)}</p>
      <p className="metric-caption">{caption}</p>
    </article>
  );
}

function Column({
  title,
  tone,
  rows,
  selectedId,
  reviewed,
  onOpen,
}: {
  title: string;
  tone: VerdictOutcome;
  rows: VerdictItem[];
  selectedId: string | null;
  reviewed: ReadonlySet<string>;
  onOpen: (id: string) => void;
}) {
  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 118,
    gap: 12,
    overscan: 4,
  });
  const items = virtualizer.getVirtualItems();
  const labelId = useMemo(() => title.toLowerCase().replace(/\s+/g, "-"), [title]);

  return (
    <section className="column" aria-labelledby={labelId}>
      <h2 id={labelId}>
        <span className={`dot dot-${tone}`} />
        {title}
        <span className="column-count">{rows.length}</span>
      </h2>
      <div className="column-scroll" ref={parentRef}>
        {rows.length === 0 ? (
          <p className="column-empty">Nothing here</p>
        ) : (
          <div className="column-virtual" style={{ height: virtualizer.getTotalSize() }}>
            {items.map((item) => {
              const verdict = rows[item.index];
              if (!verdict) return null;
              const selected = verdict.id === selectedId;
              return (
                <div
                  key={verdict.id}
                  data-index={item.index}
                  ref={virtualizer.measureElement}
                  className="card-slot"
                  style={{ transform: `translateY(${item.start}px)` }}
                >
                  <button
                    type="button"
                    className={selected ? "card is-selected" : "card"}
                    data-testid={`verdict-${verdict.id}`}
                    aria-pressed={selected}
                    onClick={() => onOpen(verdict.id)}
                  >
                    <span className="mono card-id" title={verdict.id}>
                      {shortId(verdict.id)}
                      {bindingLabel(verdict) ? ` · ${bindingLabel(verdict)}` : ""}
                    </span>
                    <span className="card-title">{cardTitle(verdict)}</span>
                    <span className="card-subtitle">{cardSubtitle(verdict)}</span>
                    <span className="card-facts">
                      <span>{sourceLabel(verdict)}</span>
                      <span>{amountLabel(verdict.amountCents)}</span>
                      <span>{ageLabel(verdict.recordedAt, Date.now())}</span>
                      {reviewed.has(verdict.id) ? <span>Reviewed</span> : null}
                    </span>
                  </button>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </section>
  );
}

export function parseOutcomeSearch(value: unknown): VerdictOutcome | undefined {
  return isOutcome(value) ? value : undefined;
}
