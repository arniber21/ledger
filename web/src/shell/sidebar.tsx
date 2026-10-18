import { Link, useRouterState } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import { countNeedsReview, countSavedInbox, savedInboxSearch } from "../inbox/model";
import { useTriage } from "../inbox/use-triage";
import { useSpecRuns } from "../specs/runs";
import { attentionCount, countSavedView } from "../stream/reduce";
import { SAVED_VIEWS, type SavedView } from "../stream/types";
import { useShell } from "./context";
import {
  ChevronIcon,
  DashboardIcon,
  InboxIcon,
  Mark,
  SearchIcon,
  SemanticIcon,
  SpecsIcon,
  TracesIcon,
  VerdictsIcon,
} from "./icons";

const NAV = [
  { to: "/inbox", label: "Inbox", icon: InboxIcon, testId: "nav-inbox", exact: true },
  { to: "/", label: "Dashboard", icon: DashboardIcon, testId: "nav-dashboard", exact: true },
  { to: "/traces", label: "Traces", icon: TracesIcon, testId: "nav-traces", exact: false },
  { to: "/taint", label: "Semantic", icon: SemanticIcon, testId: "nav-taint", exact: true },
  { to: "/specs", label: "Specs", icon: SpecsIcon, testId: "nav-specs", exact: false },
  { to: "/verdicts", label: "Verdicts", icon: VerdictsIcon, testId: "nav-verdicts", exact: false },
] as const;

export function Sidebar() {
  const { metrics, openPalette, reviewed, savedView, collapsed } = useShell();
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const search = useRouterState({ select: (state) => state.location.search });
  const triage = useTriage();
  const onSpecs = pathname === "/specs";
  const onInbox = pathname === "/inbox";
  const armed = useSpecRuns().filter((run) => run.status === "armed");
  const specParam = specFromSearch(search);
  const attention = countNeedsReview(metrics.verdicts, triage, reviewed);
  const verdictCount = attentionCount(metrics.verdicts);
  const [workspaceOpen, setWorkspaceOpen] = useState(false);
  const workspaceRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!workspaceOpen) return;
    const onPointer = (event: PointerEvent) => {
      if (!workspaceRef.current?.contains(event.target as Node)) setWorkspaceOpen(false);
    };
    window.addEventListener("pointerdown", onPointer);
    return () => window.removeEventListener("pointerdown", onPointer);
  }, [workspaceOpen]);

  return (
    <aside className="sidebar" aria-label="Sidebar" hidden={collapsed}>
      <div className="workspace" ref={workspaceRef}>
        <button
          type="button"
          className="workspace-button"
          data-testid="workspace-switcher"
          aria-expanded={workspaceOpen}
          onClick={() => setWorkspaceOpen((open) => !open)}
        >
          <Mark />
          <span>Hearthloaf</span>
          <ChevronIcon />
        </button>
        {workspaceOpen ? (
          <div className="workspace-menu" role="menu">
            <button type="button" role="menuitem" onClick={() => setWorkspaceOpen(false)}>
              <span>Hearthloaf Bakeries</span>
              <span className="mono">company-001</span>
            </button>
          </div>
        ) : null}
      </div>

      <button type="button" className="quick" data-testid="quick-actions" onClick={openPalette}>
        <SearchIcon />
        <span>Quick actions</span>
        <kbd>⌘K</kbd>
      </button>

      <nav className="nav" aria-label="Views">
        {NAV.map((item) => {
          const Icon = item.icon;
          const count = item.to === "/inbox" ? attention : item.to === "/verdicts" ? verdictCount : 0;
          return (
            <Link
              key={item.to}
              to={item.to}
              className="nav-item"
              activeOptions={{ exact: item.exact, includeSearch: false }}
              data-testid={item.testId}
            >
              <Icon />
              <span>{item.label}</span>
              {count > 0 ? <span className="nav-count">{count}</span> : <span className="nav-count" />}
            </Link>
          );
        })}
      </nav>

      {onSpecs ? (
        <section className="saved" aria-label="Armed specs">
          <h2>Armed specs</h2>
          {armed.length === 0 ? (
            <p className="saved-empty">None armed</p>
          ) : (
            armed.map((run) => {
              const id = run.specId ?? run.key;
              const active = specParam === id || specParam === run.key;
              return (
                <Link
                  key={run.key}
                  to="/specs"
                  search={{ spec: id }}
                  className={active ? "saved-item is-active" : "saved-item"}
                  data-testid={`armed-spec-${id}`}
                  aria-current={active ? "page" : undefined}
                >
                  <span className="swatch swatch-armed" />
                  <span className="saved-label">{run.patternId ?? "Armed spec"}</span>
                </Link>
              );
            })
          )}
        </section>
      ) : (
        <section className="saved" aria-label="Saved views">
          <h2>Saved views</h2>
          {SAVED_VIEWS.map((view) =>
            onInbox ? (
              <InboxSavedLink
                key={view.id}
                id={view.id}
                label={view.label}
                active={savedFromSearch(search) === view.id}
                count={countSavedInbox(metrics.verdicts, triage, view.id)}
              />
            ) : (
              <SavedLink
                key={view.id}
                id={view.id}
                label={view.label}
                active={savedView === view.id}
                count={countSavedView(metrics.verdicts, view.id)}
              />
            ),
          )}
        </section>
      )}
    </aside>
  );
}

function savedFromSearch(search: object): SavedView | undefined {
  if (!("saved" in search)) return undefined;
  const value = (search as { saved: unknown }).saved;
  if (value === "assigned" || value === "violations" || value === "unknowns" || value === "over-1000") return value;
  return undefined;
}

function specFromSearch(search: object): string | undefined {
  if (!("spec" in search)) return undefined;
  const value = (search as { spec: unknown }).spec;
  return typeof value === "string" ? value : undefined;
}

function InboxSavedLink({
  id,
  label,
  active,
  count,
}: {
  id: SavedView;
  label: string;
  active: boolean;
  count: number;
}) {
  return (
    <Link
      to="/inbox"
      search={active ? {} : savedInboxSearch(id)}
      className={active ? "saved-item is-active" : "saved-item"}
      data-testid={`saved-${id}`}
      aria-current={active ? "page" : undefined}
    >
      <span className={`swatch swatch-${id}`} />
      <span>{label}</span>
      <span className="nav-count">{count}</span>
    </Link>
  );
}

function SavedLink({
  id,
  label,
  active,
  count,
}: {
  id: SavedView;
  label: string;
  active: boolean;
  count: number;
}) {
  return (
    <Link
      to="/"
      search={{ view: active ? undefined : id, outcome: undefined }}
      className={active ? "saved-item is-active" : "saved-item"}
      data-testid={`saved-${id}`}
      aria-current={active ? "page" : undefined}
    >
      <span className={`swatch swatch-${id}`} />
      <span>{label}</span>
      <span className="nav-count">{count}</span>
    </Link>
  );
}
