import { Command } from "cmdk";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import { useSpecRuns } from "../specs/runs";
import { FAULTS, useFaultControl } from "./faults";
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

const RECENT_KEY = "ledger.palette.recent";
const RECENT_MAX = 5;

function readRecent(): string[] {
  try {
    const raw: unknown = JSON.parse(window.localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(raw) ? raw.filter((item): item is string => typeof item === "string").slice(0, RECENT_MAX) : [];
  } catch {
    return [];
  }
}

export function pushRecent(current: readonly string[], id: string): string[] {
  return [id, ...current.filter((item) => item !== id)].slice(0, RECENT_MAX);
}

function writeRecent(ids: readonly string[]) {
  try {
    window.localStorage.setItem(RECENT_KEY, JSON.stringify(ids));
  } catch {
    // Storage can be blocked. Recent items are a convenience only.
  }
}

const GO = [
  { to: "/", label: "Dashboard", keys: "G D" },
  { to: "/inbox", label: "Inbox", keys: "G I" },
  { to: "/traces", label: "Traces", keys: "G T" },
  { to: "/taint", label: "Semantic", keys: "G P" },
  { to: "/specs", label: "Specs", keys: "G S" },
  { to: "/verdicts", label: "Verdicts", keys: "G V" },
] as const;

export function Palette() {
  const {
    paletteOpen,
    closePalette,
    metrics,
    reportNotice,
    setSelectedId,
    selectedId,
    toggleReviewed,
    setCollapsed,
  } = useShell();
  const specRuns = useSpecRuns().filter((run) => run.status === "armed");
  const [recent, setRecent] = useState<string[]>(readRecent);
  const remember = (id: string) => {
    setRecent((current) => {
      const next = pushRecent(current, id);
      writeRecent(next);
      return next;
    });
  };
  const faults = useFaultControl();
  const confirmRef = useRef<HTMLButtonElement>(null);
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState(false);
  const [confirmFault, setConfirmFault] = useState<FaultName | null>(null);
  const [missing, setMissing] = useState(false);
  const [exchange, setExchange] = useState<QuickReply | null>(null);

  const go = (to: "/" | "/inbox" | "/traces" | "/taint" | "/specs" | "/verdicts") => {
    remember(`go:${to}`);
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

  const confirm = (fault: FaultName) => {
    if (faults.pending !== null) return;
    remember(`fault:${fault}`);
    setConfirmFault(null);
    closePalette();
    setQuery("");
    void faults.toggle(fault);
  };

  useEffect(() => {
    if (confirmFault) confirmRef.current?.focus();
  }, [confirmFault]);

  const lines = exchange ? exchangeLines(exchange) : [];

  return (
    <Command.Dialog
      open={paletteOpen}
      onOpenChange={(open) => {
        if (!open && confirmFault) {
          // Esc backs out of the confirm step first, then closes.
          setConfirmFault(null);
          return;
        }
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
          <p className="palette-confirm-title">Inject {confirmFault} fault?</p>
          <p>This toggles that fault for the next cycle. It does not create a verdict. A payment that carries it needs review.</p>
          <div className="palette-confirm-actions">
            <button
              ref={confirmRef}
              type="button"
              className="primary"
              data-testid="palette-confirm-fault"
              disabled={faults.pending !== null}
              onClick={() => confirm(confirmFault)}
            >
              Confirm <kbd>↵</kbd>
            </button>
            <button type="button" className="tool" data-testid="palette-cancel-fault" onClick={() => setConfirmFault(null)}>
              Cancel <kbd>Esc</kbd>
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
          {!query.trim() && recent.length > 0 ? (
            <Command.Group heading="Recent">
              {recent.map((id) => {
                const entry = recentEntry(id);
                if (!entry) return null;
                return (
                  <Command.Item
                    key={`recent-${id}`}
                    value={`recent ${entry.label}`}
                    data-testid="palette-result"
                    onSelect={() => (entry.kind === "go" ? go(entry.to) : setConfirmFault(entry.fault))}
                  >
                    <span className="palette-icon-gap" />
                    <span className="palette-title">{entry.label}</span>
                    <span className="palette-hint">Recent</span>
                  </Command.Item>
                );
              })}
            </Command.Group>
          ) : null}
          <Command.Group heading="Navigate">
            {GO.map((item) => (
              <Command.Item key={item.to} value={`Go to ${item.label}`} data-testid="palette-result" onSelect={() => go(item.to)}>
                {item.to === "/traces" ? <TracesIcon /> : <span className="palette-icon-gap" />}
                <span className="palette-title">{item.label}</span>
                {item.keys ? <kbd>{item.keys}</kbd> : null}
              </Command.Item>
            ))}
          </Command.Group>
          <Command.Group heading="Faults">
            {FAULTS.map((fault) => (
              <Command.Item
                key={fault}
                value={`Inject a ${fault} fault`}
                data-testid="palette-result"
                onSelect={() => setConfirmFault(fault)}
              >
                <BoltIcon />
                <span className="palette-title">Inject a {fault} fault</span>
                <span className="palette-hint">Confirm</span>
              </Command.Item>
            ))}
          </Command.Group>
          <Command.Group heading="Actions">
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
            <Command.Item
              value="Toggle sidebar"
              data-testid="palette-result"
              onSelect={() => {
                setCollapsed((current) => !current);
                closePalette();
              }}
            >
              <span className="palette-icon-gap" />
              <span className="palette-title">Toggle the sidebar</span>
              <kbd>[</kbd>
            </Command.Item>
            <Command.Item
              value="Keyboard shortcuts help"
              data-testid="palette-result"
              onSelect={() => {
                closePalette();
                window.setTimeout(() => window.dispatchEvent(new Event("ledger:shortcuts")), 0);
              }}
            >
              <span className="palette-icon-gap" />
              <span className="palette-title">Keyboard shortcuts</span>
              <kbd>?</kbd>
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
          <Command.Group heading="Specs">
            <Command.Item
              value="New spec"
              data-testid="palette-result"
              onSelect={() => {
                closePalette();
                void navigate({ to: "/specs", search: { spec: undefined } });
              }}
            >
              <span className="palette-icon-gap" />
              <span className="palette-title">Open specs</span>
              <kbd>G S</kbd>
            </Command.Item>
            {specRuns.map((run) => {
              const id = run.specId ?? run.key;
              return (
                <Command.Item
                  key={run.key}
                  value={`spec ${id} ${run.sentence}`}
                  data-testid="palette-result"
                  onSelect={() => {
                    closePalette();
                    void navigate({ to: "/specs", search: { spec: id } });
                  }}
                >
                  <span className="swatch swatch-armed" />
                  <span className="palette-title">{run.patternId ?? id}</span>
                  <span className="palette-open">Open</span>
                </Command.Item>
              );
            })}
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

type RecentEntry =
  | { kind: "go"; to: (typeof GO)[number]["to"]; label: string }
  | { kind: "fault"; fault: FaultName; label: string };

function recentEntry(id: string): RecentEntry | null {
  if (id.startsWith("go:")) {
    const item = GO.find((entry) => `go:${entry.to}` === id);
    return item ? { kind: "go", to: item.to, label: `Go to ${item.label}` } : null;
  }
  if (id.startsWith("fault:")) {
    const fault = FAULTS.find((name) => `fault:${name}` === id);
    return fault ? { kind: "fault", fault, label: `Inject a ${fault} fault` } : null;
  }
  return null;
}
