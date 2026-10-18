import { Outlet, useNavigate, useRouter, useRouterState } from "@tanstack/react-router";
import { useCallback, useEffect, useRef, useState } from "react";
import { inboxBridge } from "../inbox/bridge";
import { mergeInboxSearch, rememberInboxView } from "../inbox/model";
import { isSavedView } from "../stream/reduce";
import type { InboxView, SavedView } from "../stream/types";
import { useMetrics } from "../stream/use-metrics";
import { ShellProvider, type ShellContextValue } from "./context";
import { Palette } from "./palette";
import { ConfirmProvider, ShortcutsHelp, ToastProvider, useHotkeys } from "../ui";
import { Sidebar } from "./sidebar";

const TITLES: Record<string, string> = {
  "/": "Dashboard",
  "/inbox": "Inbox",
  "/traces": "Traces",
  "/taint": "Semantic",
  "/specs": "Specs",
  "/verdicts": "Verdicts",
};

export function titleFor(pathname: string): string {
  const page = TITLES[pathname.length > 1 ? pathname.replace(/\/$/, "") : pathname];
  return page ? `${page} · Ledger Sentinel` : "Not found · Ledger Sentinel";
}

const COLLAPSE_KEY = "ledger.sidebar.collapsed";

function readCollapsed(): boolean {
  try {
    return window.localStorage.getItem(COLLAPSE_KEY) === "1";
  } catch {
    return false;
  }
}

function writeCollapsed(value: boolean) {
  try {
    window.localStorage.setItem(COLLAPSE_KEY, value ? "1" : "0");
  } catch {
    // Storage can be blocked. The toggle still works for this session.
  }
}

function focusMain(event: { preventDefault: () => void }) {
  event.preventDefault();
  document.getElementById("main")?.focus();
}

function isTyping(event: KeyboardEvent): boolean {
  const target = event.target;
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || target.isContentEditable;
}

export function Shell() {
  const metrics = useMetrics();
  const navigate = useNavigate();
  const router = useRouter();
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const search = useRouterState({ select: (state) => state.location.search });
  const savedView: SavedView | undefined = isSavedView(search.view) ? search.view : undefined;

  const [paletteOpen, setPaletteOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [faultNotice, setFaultNotice] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [reviewed, setReviewed] = useState<ReadonlySet<string>>(() => new Set());
  const [inboxView, setInboxView] = useState<InboxView>("board");
  const feedRef = useRef<readonly string[]>([]);
  const [collapsed, setCollapsedState] = useState<boolean>(readCollapsed);
  const [everLive, setEverLive] = useState(false);

  const setCollapsed = useCallback((next: boolean | ((current: boolean) => boolean)) => {
    setCollapsedState((current) => {
      const value = typeof next === "function" ? next(current) : next;
      writeCollapsed(value);
      return value;
    });
  }, []);

  useEffect(() => {
    if (metrics.connection === "live") setEverLive(true);
  }, [metrics.connection]);

  useEffect(() => {
    document.title = titleFor(pathname);
  }, [pathname]);

  const registerFeed = useCallback((ids: readonly string[]) => {
    feedRef.current = ids;
  }, []);

  const toggleReviewed = useCallback((id: string) => {
    setReviewed((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const value: ShellContextValue = {
    metrics,
    paletteOpen,
    openPalette: () => setPaletteOpen(true),
    closePalette: () => setPaletteOpen(false),
    shortcutsOpen,
    setShortcutsOpen,
    faultNotice,
    reportFaultUnavailable: () => setFaultNotice("Fault injection is not connected."),
    reportNotice: (message) => setFaultNotice(message),
    selectedId,
    setSelectedId,
    registerFeed,
    feedIds: () => feedRef.current,
    reviewed,
    toggleReviewed,
    inboxView,
    setInboxView,
    savedView,
    collapsed,
    setCollapsed,
    everLive,
  };

  useEffect(() => {
    if (!faultNotice) return;
    const timer = window.setTimeout(() => setFaultNotice(null), 4000);
    return () => window.clearTimeout(timer);
  }, [faultNotice]);

  useHotkeys(
    {
      "g d": { handler: () => void navigate({ to: "/", search: { view: undefined, outcome: undefined } }), description: "Go to Dashboard" },
      "g i": { handler: () => void navigate({ to: "/inbox" }), description: "Go to Inbox" },
      "g t": { handler: () => void navigate({ to: "/traces", search: { trace: undefined } }), description: "Go to Traces" },
      "g p": { handler: () => void navigate({ to: "/taint" }), description: "Go to Taint" },
      "g s": { handler: () => void navigate({ to: "/specs", search: { spec: undefined } }), description: "Go to Specs" },
      "g v": { handler: () => void navigate({ to: "/verdicts", search: { id: undefined } }), description: "Go to Verdicts" },
    },
    { scope: "Navigate", enabled: !paletteOpen },
  );

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "k" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        setPaletteOpen(true);
        return;
      }
      const inbox = pathname === "/inbox" ? inboxBridge.current : null;
      if (inbox?.assignOpen) {
        const assigning =
          event.key === "Escape" ||
          event.key === "Enter" ||
          (!isTyping(event) && (event.key === "0" || event.key === "1"));
        if (assigning) {
          inbox.onAssignKey(event);
          return;
        }
      }
      if (isTyping(event) || paletteOpen) return;

      if (event.key === "Escape") {
        // The fault menu on Traces handles its own Esc (back to the list).
        if (event.target instanceof HTMLElement && event.target.closest(".faults")) return;
        if (shortcutsOpen) {
          setShortcutsOpen(false);
          return;
        }
        // Enter opens a verdict on its own route. Esc returns to the inbox.
        // Clearing the selection first left the verdict page in place.
        if (pathname === "/verdicts") {
          router.history.back();
          return;
        }
        if (selectedId) {
          setSelectedId(null);
          return;
        }
        router.history.back();
        return;
      }

      if (shortcutsOpen) return;

      const key = event.key.toLowerCase();

      if (pathname === "/inbox" && (key === "1" || key === "2" || key === "3")) {
        const view: InboxView = key === "1" ? "board" : key === "2" ? "list" : "split";
        event.preventDefault();
        rememberInboxView(view);
        setInboxView(view);
        void navigate({ to: "/inbox", search: (previous) => mergeInboxSearch(previous, { view }) });
        return;
      }

      if (pathname === "/inbox" && event.key === "/") {
        event.preventDefault();
        inboxBridge.current?.focusFilter();
        return;
      }

      if (pathname === "/inbox" && key === "d") {
        event.preventDefault();
        inboxBridge.current?.dismiss();
        return;
      }

      if (pathname === "/inbox" && key === "a") {
        event.preventDefault();
        inboxBridge.current?.openAssign();
        return;
      }

      if (pathname === "/inbox" && key === "e") {
        event.preventDefault();
        inboxBridge.current?.markReviewed();
        return;
      }

      const movingDown = key === "j" || (pathname === "/inbox" && event.key === "ArrowDown");
      const movingUp = key === "k" || (pathname === "/inbox" && event.key === "ArrowUp");
      if (movingDown || movingUp) {
        const ids = feedRef.current;
        if (ids.length === 0) return;
        event.preventDefault();
        const index = selectedId ? ids.indexOf(selectedId) : -1;
        const delta = movingDown ? 1 : -1;
        const nextIndex =
          index === -1 ? (delta > 0 ? 0 : ids.length - 1) : (index + delta + ids.length) % ids.length;
        const next = ids[nextIndex] ?? null;
        setSelectedId(next);
        if (pathname === "/verdicts" && next) {
          void navigate({ to: "/verdicts", search: { id: next } });
        }
        return;
      }

      if (key === "enter" && selectedId && pathname !== "/specs") {
        event.preventDefault();
        if (pathname === "/traces") {
          void navigate({ to: "/traces", search: { trace: selectedId } });
        } else {
          void navigate({ to: "/verdicts", search: { id: selectedId } });
        }
        return;
      }

      if (key === "e" && selectedId) {
        event.preventDefault();
        toggleReviewed(selectedId);
      }
    };

    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [navigate, setCollapsed, paletteOpen, pathname, router.history, selectedId, shortcutsOpen, toggleReviewed]);

  return (
    <ToastProvider>
    <ConfirmProvider>
    <ShellProvider value={value}>
      <div className="shell" data-testid="shell">
        <a className="skip-link" href="#main" onClick={focusMain}>
          Skip to content
        </a>
        <div className={collapsed ? "window is-collapsed" : "window"}>
          <Sidebar />
          <div className="main" id="main" tabIndex={-1}>
            {faultNotice ? (
              <p className="notice" role="status">
                {faultNotice}
              </p>
            ) : null}
            {metrics.connection === "offline" ? (
              <p className="notice notice-stale" role="status" data-testid="stream-banner">
                {everLive
                  ? "Reconnecting… Showing the last data received. Numbers may be out of date."
                  : "Connecting to the stream…"}
              </p>
            ) : null}
            <div className="route-fade" key={pathname}>
              <Outlet />
            </div>
          </div>
        </div>
      </div>
      <Palette />
      <ShortcutsHelp />
    </ShellProvider>
    </ConfirmProvider>
    </ToastProvider>
  );
}
