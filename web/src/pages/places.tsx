import { useNavigate } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { useEffect } from "react";
import { outcomeLabel } from "../stream/reduce";
import type { InboxView, VerdictItem } from "../stream/types";
import { useShell } from "../shell/context";
import { Header } from "../shell/header";
import {
  BoardIcon,
  FilterIcon,
  InboxIcon,
  ListIcon,
  SplitIcon,
  VerdictsIcon,
} from "../shell/icons";

export function InboxPage() {
  const { metrics, inboxView, setInboxView, openPalette, reviewed, registerFeed, selectedId, setSelectedId } =
    useShell();
  const navigate = useNavigate();
  const items = metrics.verdicts.filter(
    (verdict) =>
      (verdict.outcome === "Violate" || verdict.outcome === "Inconclusive") && !reviewed.has(verdict.id),
  );
  const feedKey = items.map((item) => item.id).join("|");

  useEffect(() => {
    registerFeed(feedKey.length === 0 ? [] : feedKey.split("|"));
    return () => registerFeed([]);
  }, [feedKey, registerFeed]);

  const meta = items.length === 0 ? "Nothing needs attention" : `${items.length} need attention`;

  return (
    <>
      <Header
        icon={<InboxIcon />}
        title="Inbox"
        meta={meta}
        actions={
          <div className="inbox-tools">
            <button type="button" className="tool" onClick={openPalette}>
              <FilterIcon />
              Filter
            </button>
            <div className="views" role="group" aria-label="Inbox view">
              <ViewButton id="board" label="Board" icon={<BoardIcon />} current={inboxView} onSelect={setInboxView} />
              <ViewButton id="list" label="List" icon={<ListIcon />} current={inboxView} onSelect={setInboxView} />
              <ViewButton id="split" label="Split" icon={<SplitIcon />} current={inboxView} onSelect={setInboxView} />
            </div>
          </div>
        }
      />
      <div className="content content-fill" data-view={inboxView}>
        {items.length === 0 ? (
          <Empty
            title="Nothing needs review"
            body="Violate and inconclusive verdicts show up here when the stream sends them."
          />
        ) : inboxView === "list" ? (
          <ul className="feed">
            {items.map((item) => (
              <li key={item.id}>
                <VerdictRow item={item} selected={item.id === selectedId} onOpen={() => openItem(item.id)} />
              </li>
            ))}
          </ul>
        ) : (
          <div className={inboxView === "split" ? "split" : "inbox-board"}>
            <ul className="feed">
              {items.map((item) => (
                <li key={item.id}>
                  <VerdictRow item={item} selected={item.id === selectedId} onOpen={() => openItem(item.id)} />
                </li>
              ))}
            </ul>
            {inboxView === "split" ? <SplitDetail items={items} selectedId={selectedId} /> : null}
          </div>
        )}
      </div>
      <footer className="hintbar">
        <span>J and K move between items · Enter opens · E marks reviewed</span>
        <span>1, 2, 3 switch view</span>
      </footer>
    </>
  );

  function openItem(id: string) {
    setSelectedId(id);
    void navigate({ to: "/verdicts", search: { id } });
  }
}

function ViewButton({
  id,
  label,
  icon,
  current,
  onSelect,
}: {
  id: InboxView;
  label: string;
  icon: ReactNode;
  current: InboxView;
  onSelect: (view: InboxView) => void;
}) {
  return (
    <button
      type="button"
      data-testid={`view-${id}`}
      aria-pressed={current === id}
      onClick={() => onSelect(id)}
    >
      {icon}
      {label}
    </button>
  );
}

function VerdictRow({
  item,
  selected,
  onOpen,
}: {
  item: VerdictItem;
  selected: boolean;
  onOpen: () => void;
}) {
  return (
    <button
      type="button"
      className={selected ? "row is-selected" : "row"}
      data-testid={`inbox-item-${item.id}`}
      onClick={onOpen}
    >
      <span className={`dot dot-${item.outcome}`} />
      <span className="mono">{item.id}</span>
      <span className="row-title">{item.title}</span>
      <span className="row-meta">{item.evidence || outcomeLabel(item.outcome)}</span>
    </button>
  );
}

function SplitDetail({ items, selectedId }: { items: VerdictItem[]; selectedId: string | null }) {
  const item = items.find((entry) => entry.id === selectedId) ?? items[0];
  if (!item) return null;
  return (
    <article className="split-detail">
      <p className="mono">{item.id}</p>
      <h2>{item.title}</h2>
      <p className={`outcome outcome-${item.outcome}`}>{outcomeLabel(item.outcome)}</p>
      <p>{item.evidence || "No evidence on this verdict."}</p>
    </article>
  );
}

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
