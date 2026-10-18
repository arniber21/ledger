import { outcomeLabel } from "../stream/reduce";
import { useShell } from "../shell/context";
import { Header } from "../shell/header";
import { VerdictsIcon } from "../shell/icons";

export function VerdictsPage({ id }: { id?: string }) {
  const { metrics } = useShell();
  const verdict = id ? metrics.verdicts.find((item) => item.id === id) : undefined;

  return (
    <>
      <Header icon={<VerdictsIcon />} title="Verdicts" />
      <div className="content content-fill">
        {verdict ? (
          <article className="identity" data-testid="verdict-identity">
            <p className="mono">{verdict.id}</p>
            <h2>{verdict.title}</h2>
            <p className={`outcome outcome-${verdict.outcome}`}>{outcomeLabel(verdict.outcome)}</p>
            <p>{verdict.evidence || "No evidence on this verdict."}</p>
            {verdict.reasonCode ? <p className="mono reason">{verdict.reasonCode}</p> : null}
          </article>
        ) : (
          <Empty
            title={id ? "Verdict is not on the stream" : "No verdicts yet"}
            body={
              id
                ? `${id} is not in the current snapshot.`
                : "Satisfy, Violate, and Inconclusive outcomes show up here. A missing signal stays inconclusive."
            }
          />
        )}
      </div>
    </>
  );
}

function Empty({ title, body }: { title: string; body: string }) {
  return (
    <div className="empty">
      <h2>{title}</h2>
      <p>{body}</p>
    </div>
  );
}
