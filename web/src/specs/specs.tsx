import { tableFeatures, useTable, type ColumnDef } from "@tanstack/react-table";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useRef, useState, type FormEvent } from "react";
import type { VerdictItem } from "../stream/types";
import { useShell } from "../shell/context";
import { Header } from "../shell/header";
import { SpecsIcon } from "../shell/icons";
import {
  applyAuthorEvent,
  beginDraft,
  checkDetail,
  composingRun,
  coverageSummary,
  coverageWord,
  failDraft,
  finishDraft,
  policySegments,
  scopeOf,
  stepTitle,
} from "./reduce";
import { insertRun, replaceRun, useSpecRuns } from "./runs";
import { streamAuthoring } from "./stream";
import { JEV_THRESHOLD_LABEL, type SpecRun, type SpecStatus, type TimelineStep } from "./types";

const features = tableFeatures({});
type Features = typeof features;
type TableRow = { id: string; pattern: string; scope: string; status: string; detail: string };

const columns: ColumnDef<Features, TableRow>[] = [
  { accessorKey: "pattern", header: "Pattern" },
  { accessorKey: "scope", header: "Scope" },
  { accessorKey: "status", header: "Status" },
  { accessorKey: "detail", header: "Detail" },
];

export function SpecsPage({ spec }: { spec?: string }) {
  const runs = useSpecRuns();
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const { metrics } = useShell();
  const selected = runs.find((run) => run.key === spec || run.specId === spec);
  const armed = runs.filter((run) => run.status === "armed");
  const meta = armed.length === 0 ? "None armed" : `${armed.length} armed`;

  const onNew = () => {
    const open = runs.find((run) => run.status === "composing");
    const run = open ?? composingRun(crypto.randomUUID());
    if (!open) insertRun(queryClient, run);
    void navigate({ to: "/specs", search: { spec: run.key } });
  };

  return (
    <>
      <Header
        icon={<SpecsIcon />}
        title="Specs"
        meta={meta}
        actions={
          <button type="button" className="primary" data-testid="new-spec" onClick={onNew}>
            New spec
          </button>
        }
      />
      {selected ? (
        <SpecDetail key={selected.key} run={selected} runs={runs} verdicts={metrics.verdicts} />
      ) : (
        <div className="content content-fill">
          <div className="empty">
            <h2>No specs yet</h2>
            <p>A policy sentence is drafted here, then checked, and armed only when the authoring stream accepts it.</p>
          </div>
        </div>
      )}
    </>
  );
}

function SpecDetail({
  run,
  runs,
  verdicts,
}: {
  run: SpecRun;
  runs: SpecRun[];
  verdicts: VerdictItem[];
}) {
  const queryClient = useQueryClient();
  const [tab, setTab] = useState<"authoring" | "monitors" | "history">("authoring");
  const [sentence, setSentence] = useState(run.sentence);
  const abortRef = useRef<AbortController | null>(null);
  const editable = run.status === "composing" || run.status === "unavailable";
  const counts = verdictCounts(verdicts, run.specId);
  const scope = scopeOf(run.parameters);
  const segments = policySegments(run.sentence, run.clauses);

  useEffect(() => () => abortRef.current?.abort(), []);

  const onSubmit = (event: FormEvent) => {
    event.preventDefault();
    const text = sentence.trim();
    if (text.length === 0) {
      replaceRun(queryClient, run.key, (current) => failDraft(beginDraft(current, ""), "Enter a policy sentence."));
      return;
    }
    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;
    replaceRun(queryClient, run.key, (current) => beginDraft(current, text));
    void streamAuthoring(
      text,
      (authorEvent) => {
        replaceRun(queryClient, run.key, (current) => applyAuthorEvent(current, authorEvent));
      },
      controller.signal,
      false,
    )
      .then(() => {
        replaceRun(queryClient, run.key, finishDraft);
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        const message = error instanceof Error ? error.message : "The authoring stream failed.";
        replaceRun(queryClient, run.key, (current) => failDraft(current, message));
      });
  };

  const onOverride = () => {
    const text = run.sentence.trim();
    if (text.length === 0 || run.status !== "held") return;
    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;
    replaceRun(queryClient, run.key, (current) => beginDraft(current, text));
    void streamAuthoring(
      text,
      (authorEvent) => {
        replaceRun(queryClient, run.key, (current) => applyAuthorEvent(current, authorEvent));
      },
      controller.signal,
      true,
    )
      .then(() => {
        replaceRun(queryClient, run.key, finishDraft);
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        const message = error instanceof Error ? error.message : "The authoring stream failed.";
        replaceRun(queryClient, run.key, (current) => failDraft(current, message));
      });
  };

  return (
    <div className="content">
      <div className="spec-detail">
        <section className="spec-identity" aria-label="Spec">
          <div className="spec-tile">
            <SpecsIcon />
          </div>
          <p className="mono spec-id">{run.specId ?? "—"}</p>
          <div className="spec-heading">
            <h2>{run.patternId ?? "New spec"}</h2>
            <StatusChip status={run.status} />
          </div>
          {editable ? (
            <form onSubmit={onSubmit}>
              <label className="spec-label" htmlFor="spec-policy">
                Policy
              </label>
              <textarea
                id="spec-policy"
                data-testid="spec-policy"
                className="spec-field"
                value={sentence}
                placeholder="Write the policy in plain English"
                onChange={(event) => setSentence(event.target.value)}
              />
              <button type="submit" className="tool spec-submit" data-testid="draft-spec" disabled={run.status === "drafting"}>
                Draft spec
              </button>
            </form>
          ) : (
            <>
              <p className="spec-policy" data-testid="spec-policy">
                <PolicyText sentence={run.sentence} segments={segments} />
              </p>
              {run.status === "held" ? (
                <button type="button" className="tool spec-submit" data-testid="override-arm" onClick={onOverride}>
                  Arm with override
                </button>
              ) : null}
            </>
          )}
          <dl className="spec-props">
            <dt>Pattern</dt>
            <dd>{run.patternId ?? "—"}</dd>
            <dt>Scope</dt>
            <dd>{scope ?? "—"}</dd>
            <dt>Status</dt>
            <dd>{statusLabel(run.status)}</dd>
            <dt>Monitors</dt>
            <dd>{run.monitor === "running" ? "1 running" : "0"}</dd>
            <dt>Verdicts</dt>
            <dd>{`${counts.satisfy} satisfied · ${counts.violate} needs review · ${counts.inconclusive} inconclusive`}</dd>
            <dt>Drafted by</dt>
            <dd>{run.draftedBy ?? "—"}</dd>
            <dt>Jev threshold</dt>
            <dd>{JEV_THRESHOLD_LABEL}</dd>
            <dt>Clause coverage</dt>
            <dd data-testid="clause-coverage">{coverageSummary(segments)}</dd>
          </dl>
        </section>

        <section aria-label="Spec timeline">
          <div className="spec-tabs" role="tablist" aria-label="Spec sections">
            <Tab id="authoring" label="Authoring" current={tab} onSelect={setTab} />
            <Tab id="monitors" label="Monitors" current={tab} onSelect={setTab} />
            <Tab id="history" label="History" current={tab} onSelect={setTab} />
          </div>
          {tab === "authoring" ? <Authoring run={run} /> : null}
          {tab === "monitors" ? <MonitorTable run={run} scope={scope} /> : null}
          {tab === "history" ? <HistoryTable runs={runs} currentKey={run.key} /> : null}
        </section>
      </div>
    </div>
  );
}

function Authoring({ run }: { run: SpecRun }) {
  return (
    <div role="tabpanel">
      {run.error ? (
        <p className="spec-error" role="alert" data-testid="spec-error">
          {run.error}
        </p>
      ) : null}
      {run.steps.length === 0 && !run.error ? (
        <p className="column-empty spec-wait">
          {run.status === "drafting"
            ? "Waiting for the authoring stream."
            : "Steps show up here as the authoring stream sends them."}
        </p>
      ) : null}
      {run.steps.length > 0 ? (
        <ol className="timeline">
          {run.steps.map((step, index) => (
            <li
              key={`${step.kind}-${index}`}
              className={index === run.steps.length - 1 && run.status === "armed" ? "timeline-event is-live" : "timeline-event"}
              data-testid="timeline-event"
            >
              <div className="timeline-mark" aria-hidden="true">
                <i />
                <span />
              </div>
              <div className="timeline-body">
                <p className="timeline-kicker">{stepTitle(step)}</p>
                <StepBody step={step} />
              </div>
            </li>
          ))}
        </ol>
      ) : null}
    </div>
  );
}

function StepBody({ step }: { step: TimelineStep }) {
  switch (step.kind) {
    case "policy":
      return <p className="timeline-policy">{step.text}</p>;
    case "draft":
      return <CodeCard lines={step.lines} />;
    case "catalogue":
      return step.detail ? <p className="timeline-detail">{step.detail}</p> : null;
    case "check":
      return <p className="timeline-detail">{checkDetail(step.verdict)}</p>;
    case "coverage":
      return (
        <div className="timeline-detail">
          {step.clauses.map((clause, index) => (
            <p key={`${clause.coverage}-${index}`}>
              {clause.text.trim()} · {coverageWord(clause.coverage)}
              {clause.verdict ? ` · ${clause.verdict}` : ""} · threshold {JEV_THRESHOLD_LABEL}
            </p>
          ))}
        </div>
      );
    case "armed":
      return <p className="timeline-detail">{step.patternId}</p>;
    case "held":
      return <p className="timeline-detail">{step.detail}</p>;
    case "monitor":
      return <p className="timeline-detail">{step.monitor}</p>;
    case "rejected":
      return <p className="timeline-detail">{step.detail}</p>;
  }
}

function CodeCard({ lines }: { lines: string[] }) {
  return (
    <div className="code-card">
      {lines.map((line, index) => (
        <div className="code-line" key={`${index}-${line}`}>
          <span>{index + 1}</span>
          <code>{line}</code>
        </div>
      ))}
    </div>
  );
}

function MonitorTable({ run, scope }: { run: SpecRun; scope: string | null }) {
  const rows: TableRow[] =
    run.monitor === "running"
      ? [
          {
            id: run.specId ?? run.key,
            pattern: run.patternId ?? "—",
            scope: scope ?? "—",
            status: "Armed",
            detail: "Running",
          },
        ]
      : [];
  return (
    <div role="tabpanel">
      <PlainTable rows={rows} empty="No monitor is running for this spec." />
    </div>
  );
}

function HistoryTable({ runs, currentKey }: { runs: SpecRun[]; currentKey: string }) {
  const rows: TableRow[] = runs
    .filter((run) => run.key !== currentKey && run.status !== "composing")
    .map((run) => ({
      id: run.key,
      pattern: run.patternId ?? "—",
      scope: scopeOf(run.parameters) ?? "—",
      status: statusLabel(run.status),
      detail: run.sentence || run.error || "—",
    }));
  return (
    <div role="tabpanel">
      <PlainTable rows={rows} empty="No earlier specs." />
    </div>
  );
}

function PlainTable({ rows, empty }: { rows: TableRow[]; empty: string }) {
  const table = useTable({
    features,
    data: rows,
    columns,
    getRowId: (row) => row.id,
  });

  if (rows.length === 0) return <p className="column-empty spec-wait">{empty}</p>;

  return (
    <table className="spec-table">
      <thead>
        {table.getHeaderGroups().map((group) => (
          <tr key={group.id}>
            {group.headers.map((header) => (
              <th key={header.id}>
                <table.FlexRender header={header} />
              </th>
            ))}
          </tr>
        ))}
      </thead>
      <tbody>
        {table.getRowModel().rows.map((row) => (
          <tr key={row.id}>
            {columns.map((column) => {
              const key = "accessorKey" in column ? String(column.accessorKey) : row.id;
              return <td key={key}>{row.original[key as keyof TableRow]}</td>;
            })}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Tab({
  id,
  label,
  current,
  onSelect,
}: {
  id: "authoring" | "monitors" | "history";
  label: string;
  current: "authoring" | "monitors" | "history";
  onSelect: (id: "authoring" | "monitors" | "history") => void;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={current === id}
      data-testid={`spec-tab-${id}`}
      onClick={() => onSelect(id)}
    >
      {label}
    </button>
  );
}

function StatusChip({ status }: { status: SpecStatus }) {
  if (status === "composing") return null;
  const armed = status === "armed";
  return (
    <span className={armed ? "chip" : `chip status-${status}`} data-testid="spec-status">
      {statusLabel(status)}
    </span>
  );
}

function PolicyText({
  sentence,
  segments,
}: {
  sentence: string;
  segments: ReturnType<typeof policySegments>;
}) {
  if (!segments) return sentence;
  const tight = segments.map((clause) => clause.text).join("");
  const gap = tight === sentence ? "" : " ";
  return segments.map((clause, index) => (
    <span
      key={`${clause.coverage}-${index}`}
      className={`clause clause-${clause.coverage}`}
      data-testid={`clause-${clause.coverage}`}
      title={coverageWord(clause.coverage)}
    >
      {index > 0 ? gap : ""}
      {clause.text}
    </span>
  ));
}

function statusLabel(status: SpecStatus): string {
  switch (status) {
    case "composing":
      return "Not armed";
    case "drafting":
      return "Drafting";
    case "armed":
      return "Armed";
    case "rejected":
      return "Rejected";
    case "unavailable":
      return "Unavailable";
    case "held":
      return "Needs review";
  }
}

function verdictCounts(verdicts: VerdictItem[], specId: string | null) {
  const matched = specId ? verdicts.filter((verdict) => verdict.specId === specId) : [];
  return {
    satisfy: matched.filter((verdict) => verdict.outcome === "Satisfy").length,
    violate: matched.filter((verdict) => verdict.outcome === "Violate").length,
    inconclusive: matched.filter((verdict) => verdict.outcome === "Inconclusive").length,
  };
}
