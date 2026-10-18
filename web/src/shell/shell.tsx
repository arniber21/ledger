import { Outlet, useNavigate, useRouter, useRouterState } from "@tanstack/react-router";
import { useCallback, useEffect, useRef, useState } from "react";
import { inboxBridge } from "../inbox/bridge";
import { mergeInboxSearch, rememberInboxView } from "../inbox/model";
import { isSavedView } from "../stream/reduce";
import type { InboxView, SavedView } from "../stream/types";
import { useMetrics } from "../stream/use-metrics";
import { ShellProvider, type ShellContextValue } from "./context";
import { Palette } from "./palette";
import { Shortcuts } from "./shortcuts";
import { Sidebar } from "./sidebar";

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
  };

  useEffect(() => {
    if (!faultNotice) return;
    const timer = window.setTimeout(() => setFaultNotice(null), 4000);
    return () => window.clearTimeout(timer);
  }, [faultNotice]);

  useEffect(() => {
    let chord = false;
    let timer = 0;

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
        if (shortcutsOpen) {
          setShortcutsOpen(false);
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

      if (event.key === "?") {
        event.preventDefault();
        setShortcutsOpen(true);
        return;
      }

      const key = event.key.toLowerCase();
      if (key === "g" && !event.metaKey && !event.ctrlKey && !event.altKey) {
        chord = true;
        window.clearTimeout(timer);
        timer = window.setTimeout(() => {
          chord = false;
        }, 1500);
        return;
      }

      if (chord) {
        chord = false;
        const destination = {
          d: "/",
          i: "/inbox",
          t: "/traces",
          p: "/taint",
          s: "/specs",
          v: "/verdicts",
        }[key];
        if (destination) {
          event.preventDefault();
          void navigate({ to: destination });
        }
        return;
      }

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

      if (key === "enter" && selectedId) {
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
      window.clearTimeout(timer);
    };
  }, [navigate, paletteOpen, pathname, router.history, selectedId, shortcutsOpen, toggleReviewed]);

  return (
    <ShellProvider value={value}>
      <div className="shell" data-testid="shell">
        <div className="window">
          <Sidebar />
          <div className="main">
            {faultNotice ? (
              <p className="notice" role="status">
                {faultNotice}
              </p>
            ) : null}
            <Outlet />
          </div>
        </div>
      </div>
      <Palette />
      <Shortcuts />
    </ShellProvider>
  );
}
