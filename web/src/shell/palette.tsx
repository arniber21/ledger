import { Command } from "cmdk";
import { useNavigate } from "@tanstack/react-router";
import type { VerdictOutcome } from "../stream/types";
import { useShell } from "./context";
import { BoltIcon, CheckIcon, SearchIcon, TracesIcon } from "./icons";

const GO = [
  { to: "/", label: "Dashboard", keys: "G D" },
  { to: "/inbox", label: "Inbox", keys: "" },
  { to: "/traces", label: "Traces", keys: "G T" },
  { to: "/specs", label: "Specs", keys: "G S" },
  { to: "/verdicts", label: "Verdicts", keys: "G V" },
] as const;

export function Palette() {
  const { paletteOpen, closePalette, metrics, reportFaultUnavailable, setSelectedId, selectedId, toggleReviewed } =
    useShell();
  const navigate = useNavigate();

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

  return (
    <Command.Dialog
      open={paletteOpen}
      onOpenChange={(open) => {
        if (!open) closePalette();
      }}
      label="Quick actions"
      data-testid="palette"
      className="palette"
    >
      <div className="palette-input">
        <SearchIcon />
        <Command.Input data-testid="palette-input" placeholder="Search verdicts, actions and views" />
        <kbd>Esc</kbd>
      </div>
      <Command.List className="palette-list">
        <Command.Empty className="palette-empty">No matches</Command.Empty>
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
            onSelect={() => {
              closePalette();
              reportFaultUnavailable();
            }}
          >
            <BoltIcon />
            <span className="palette-title">Inject a double-pay fault</span>
            <span className="palette-hint">Agent 2</span>
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
      <div className="palette-foot">
        <span>Up and down to move · Enter to select</span>
        <span>Searching verdicts, actions and views</span>
      </div>
      </Command.Dialog>
  );
}
