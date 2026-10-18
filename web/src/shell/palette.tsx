import { Command } from "cmdk";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import type { VerdictOutcome } from "../stream/types";
import { useShell } from "./context";
import { BoltIcon, CheckIcon, SearchIcon, TracesIcon } from "./icons";
import {
  applyQuick,
  candidatesFrom,
  exchangeLines,
  routePhrase,
  type FaultName,
  type QuickReply,
} from "./quick";

const GO = [
  { to: "/", label: "Dashboard", keys: "G D" },
  { to: "/inbox", label: "Inbox", keys: "" },
  { to: "/traces", label: "Traces", keys: "G T" },
  { to: "/specs", label: "Specs", keys: "G S" },
  { to: "/verdicts", label: "Verdicts", keys: "G V" },
] as const;

export function Palette() {
  const {
    paletteOpen,
    closePalette,
    metrics,
    reportFaultUnavailable,
    reportNotice,
    setSelectedId,
    selectedId,
    toggleReviewed,
  } = useShell();
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState(false);
  const [confirmFault, setConfirmFault] = useState<FaultName | null>(null);
  const [missing, setMissing] = useState(false);
  const [exchange, setExchange] = useState<QuickReply | null>(null);

  const go = (to: "/" | "/inbox" | "/traces" | "/specs" | "/verdicts") => {
    closePalette();
    void navigate({ to });
  };

  const openVerdict = (id: string) => {
    setSelectedId(id);
    closePalette();
    void navigate({ to: "/verdicts", search: { id } });
  };

  const filterOutcome = (outcome: VerdictOutcome) => {
    closePalette();
    void navigate({ to: "/", search: { view: undefined, outcome } });
  };

  const run = async (phrase: string) => {
    const trimmed = phrase.trim();
    if (!trimmed || pending) return;
    setPending(true);
    setMissing(false);
    setConfirmFault(null);
    try {
      const reply = await routePhrase(trimmed, candidatesFrom(metrics.verdicts));
      if (!reply) {
        reportNotice("Jev returned Unknown. Showing the phrase as text search.");
        closePalette();
        void navigate({ to: "/inbox", search: { q: trimmed } });
        return;
      }
      setExchange(reply);
      const applied = applyQuick(reply, new Set(metrics.verdicts.map((verdict) => verdict.id)));
      if (applied.type === "confirm") {
        setConfirmFault(applied.fault);
        return;
      }
      if (applied.type === "missing") {
        setMissing(true);
        reportNotice(applied.notice);
        return;
      }
      reportNotice(applied.notice);
      closePalette();
      setQuery("");
      if (applied.type === "inbox" || applied.type === "text") {
        void navigate({ to: "/inbox", search: applied.search });
      } else if (applied.type === "go") {
        void navigate({ to: applied.to });
      } else if (applied.type === "open") {
        setSelectedId(applied.id);
        void navigate({ to: "/verdicts", search: { id: applied.id } });
      }
    } finally {
      setPending(false);
    }
  };

  const confirm = async (fault: FaultName) => {
    setPending(true);
    try {
      const response = await fetch(`/api/faults/${fault}`, { method: "POST" });
      if (!response.ok) {
        reportFaultUnavailable();
        return;
      }
      reportNotice(`${fault} toggled on the running desk. A payment that carries it needs review.`);
      setConfirmFault(null);
      closePalette();
    } catch {
      reportFaultUnavailable();
    } finally {
      setPending(false);
    }
  };

  const lines = exchange ? exchangeLines(exchange) : [];

  return (
    <Command.Dialog
      open={paletteOpen}
      onOpenChange={(open) => {
        if (!open) {
          closePalette();
          setConfirmFault(null);
          setMissing(false);
          setQuery("");
        }
      }}
      label="Quick actions"
      data-testid="palette"
      className="palette"
    >
      <div className="palette-input">
        <SearchIcon />
        <Command.Input
          data-testid="palette-input"
          placeholder="Search verdicts, actions and views"
          value={query}
          onValueChange={setQuery}
        />
        <kbd>Esc</kbd>
      </div>
      {confirmFault ? (
        <div className="palette-confirm" data-testid="palette-confirm">
          <p>Inject {confirmFault} on the running desk?</p>
          <p>This toggles that fault for the next cycle. It does not create a verdict. A payment that carries it needs review.</p>
          <div className="palette-confirm-actions">
            <button type="button" className="primary" data-testid="palette-confirm-fault" disabled={pending} onClick={() => void confirm(confirmFault)}>
              Confirm
            </button>
            <button type="button" className="tool" data-testid="palette-cancel-fault" onClick={() => setConfirmFault(null)}>
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <Command.List className="palette-list">
          <Command.Empty className="palette-empty">No matches</Command.Empty>
          {query.trim() ? (
            <Command.Group heading="Phrase">
              <Command.Item
                value={query}
                data-testid="palette-route"
                disabled={pending}
                onSelect={() => void run(query)}
              >
                <SearchIcon />
                <span className="palette-title">{pending ? "Routing…" : `Route “${query.trim()}”`}</span>
              </Command.Item>
            </Command.Group>
          ) : null}
          {missing ? (
            <Command.Group heading="Verdict">
              <Command.Item value="missing verdict" data-testid="palette-missing" disabled>
                <span className="palette-title">That verdict is not on the stream. Quick actions did not create one.</span>
              </Command.Item>
            </Command.Group>
          ) : null}
          {metrics.verdicts.length > 0 ? (
            <Command.Group heading="Verdicts">
              {metrics.verdicts.map((verdict) => (
                <Command.Item
                  key={verdict.id}
                  value={`${verdict.id} ${verdict.title} ${verdict.outcome}`}
                  data-testid="palette-result"
                  onSelect={() => openVerdict(verdict.id)}
                >
                  <span className={`dot dot-${verdict.outcome}`} />
                  <span className="mono palette-id">{verdict.id}</span>
                  <span className="palette-title">{verdict.title}</span>
                  <span className="palette-open">Open</span>
                </Command.Item>
              ))}
            </Command.Group>
          ) : null}
          <Command.Group heading="Actions">
            <Command.Item
              value="Inject a double-pay fault"
              data-testid="palette-result"
              onSelect={() => setConfirmFault("double-pay")}
            >
              <BoltIcon />
              <span className="palette-title">Inject a double-pay fault</span>
              <span className="palette-hint">Confirm</span>
            </Command.Item>
            <Command.Item
              value="Mark reviewed"
              data-testid="palette-result"
              onSelect={() => {
                if (selectedId) toggleReviewed(selectedId);
                closePalette();
              }}
            >
              <CheckIcon />
              <span className="palette-title">Mark the selected verdict reviewed</span>
              <kbd>E</kbd>
            </Command.Item>
          </Command.Group>
          <Command.Group heading="Filter">
            <Command.Item value="Needs review filter" data-testid="palette-result" onSelect={() => filterOutcome("Violate")}>
              <span className="dot dot-Violate" />
              <span className="palette-title">Needs review</span>
            </Command.Item>
            <Command.Item
              value="Inconclusive filter"
              data-testid="palette-result"
              onSelect={() => filterOutcome("Inconclusive")}
            >
              <span className="dot dot-Inconclusive" />
              <span className="palette-title">Inconclusive</span>
            </Command.Item>
            <Command.Item value="Satisfied filter" data-testid="palette-result" onSelect={() => filterOutcome("Satisfy")}>
              <span className="dot dot-Satisfy" />
              <span className="palette-title">Satisfied</span>
            </Command.Item>
          </Command.Group>
          <Command.Group heading="Go to">
            {GO.map((item) => (
              <Command.Item key={item.to} value={item.label} data-testid="palette-result" onSelect={() => go(item.to)}>
                {item.to === "/traces" ? <TracesIcon /> : <span className="palette-icon-gap" />}
                <span className="palette-title">{item.label}</span>
                {item.keys ? <kbd>{item.keys}</kbd> : null}
              </Command.Item>
            ))}
          </Command.Group>
        </Command.List>
      )}
      {lines.length > 0 ? (
        <ul className="palette-exchange" data-testid="palette-exchange">
          {lines.map((line) => (
            <li key={line}>{line}</li>
          ))}
        </ul>
      ) : null}
      <div className="palette-foot">
        <span>{query.trim() ? "Enter routes the phrase" : "Up and down to move · Enter to select"}</span>
        <span>Jev fills filters from known values</span>
      </div>
    </Command.Dialog>
  );
}
