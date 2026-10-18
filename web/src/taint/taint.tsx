import { useState } from "react";
import { Header } from "../shell/header";
import { SemanticIcon } from "../shell/icons";
import { RECORDED, QUESTIONS, type RecordedField } from "./recorded";

/** Fixed before any panel response was read. Matches ledger-core. */
const THRESHOLD = 0.8;

type Mark = "yes" | "no" | "unknown";

function markOf(probability: number | null): Mark {
  if (probability == null || !Number.isFinite(probability)) return "unknown";
  if (probability >= THRESHOLD) return "yes";
  if (1 - probability >= THRESHOLD) return "no";
  return "unknown";
}

function markLabel(questionId: string, mark: Mark): string {
  if (mark === "unknown") return "Inconclusive";
  if (questionId === "memo_instructs_payment" && mark === "yes") return "Needs review";
  if (mark === "yes") return "Yes";
  return "No";
}

function outcomeClass(questionId: string, mark: Mark): string {
  if (mark === "unknown") return "outcome outcome-Inconclusive";
  if (questionId === "memo_instructs_payment" && mark === "yes") return "outcome outcome-Violate";
  if (mark === "yes") return "outcome outcome-Violate";
  return "outcome outcome-Satisfy";
}

export function TaintPage() {
  const [selectedId, setSelectedId] = useState(RECORDED[0]?.id);
  const selected = RECORDED.find((field) => field.id === selectedId) ?? RECORDED[0];
  const meta = RECORDED.length === 0 ? "No recorded calls" : `${RECORDED.length} recorded calls`;

  return (
    <>
      <Header icon={<SemanticIcon />} title="Semantic" meta={meta} />
      <div className="content content-fill trace-page">
        {RECORDED.length === 0 || !selected ? (
          <div className="empty" data-testid="taint-empty">
            <h2>No semantic panel yet</h2>
            <p>One stored Jev call per free-text field shows its eight scores here. A timeout, error, or score under 0.80 is inconclusive.</p>
          </div>
        ) : (
          <div className="trace-layout">
            <ul className="feed trace-feed" data-testid="taint-fields">
              {RECORDED.map((field) => (
                <li key={field.id}>
                  <button
                    type="button"
                    className={field.id === selected.id ? "row trace-row is-selected" : "row trace-row"}
                    onClick={() => setSelectedId(field.id)}
                  >
                    <span className="trace-source">{field.source}</span>
                    <span className="trace-evidence">{field.text}</span>
                  </button>
                </li>
              ))}
            </ul>
            <PanelDetail field={selected} />
          </div>
        )}
      </div>
      <footer className="hintbar">
        <span>Eight yes/no questions in one request</span>
        <span>0.80 is fixed in code</span>
      </footer>
    </>
  );
}

function PanelDetail({ field }: { field: RecordedField }) {
  const instructs = field.questions.find((question) => question.id === "memo_instructs_payment");
  const instructsMark = markOf(instructs?.probability ?? null);

  return (
    <article className="trace-detail" data-testid="taint-panel">
      <div className="trace-identity">
        <span className="trace-tile">
          <SemanticIcon />
        </span>
        <p className="mono trace-full-id">{field.source}</p>
        <h2>{field.text}</h2>
        <p className={outcomeClass("memo_instructs_payment", instructsMark)}>
          {instructsMark === "yes"
            ? "Needs review"
            : instructsMark === "no"
              ? "Does not instruct a payment"
              : "Inconclusive"}
        </p>
        <dl className="props">
          <div className="prop">
            <dt>Questions</dt>
            <dd>{QUESTIONS.length} in 1 request</dd>
          </div>
          <div className="prop">
            <dt>Threshold</dt>
            <dd>0.80</dd>
          </div>
        </dl>
      </div>
      <div className="trace-panel">
        <div className="attr-card" data-testid="taint-scores">
          {QUESTIONS.map((question, index) => {
            const stored = field.questions.find((item) => item.id === question.id);
            const probability = stored?.probability ?? null;
            const mark = markOf(probability);
            return (
              <p key={question.id} className="attr-line">
                <span>{index + 1}</span>
                <span>
                  {question.label}
                  {"  "}
                  <span className="mono">{probability == null ? "unknown" : probability.toFixed(2)}</span>
                  {"  "}
                  <span className={outcomeClass(question.id, mark)}>{markLabel(question.id, mark)}</span>
                </span>
              </p>
            );
          })}
        </div>
      </div>
    </article>
  );
}
