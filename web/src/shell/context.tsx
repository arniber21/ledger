import { createContext, useContext, type ReactNode } from "react";
import type { InboxView, MetricsSnapshot, SavedView } from "../stream/types";

export type ShellContextValue = {
  metrics: MetricsSnapshot;
  paletteOpen: boolean;
  openPalette: () => void;
  closePalette: () => void;
  shortcutsOpen: boolean;
  setShortcutsOpen: (open: boolean) => void;
  faultNotice: string | null;
  reportFaultUnavailable: () => void;
  reportNotice: (message: string) => void;
  selectedId: string | null;
  setSelectedId: (id: string | null) => void;
  registerFeed: (ids: readonly string[]) => void;
  feedIds: () => readonly string[];
  reviewed: ReadonlySet<string>;
  toggleReviewed: (id: string) => void;
  inboxView: InboxView;
  setInboxView: (view: InboxView) => void;
  savedView: SavedView | undefined;
};

const ShellContext = createContext<ShellContextValue | null>(null);

export function ShellProvider({
  value,
  children,
}: {
  value: ShellContextValue;
  children: ReactNode;
}) {
  return <ShellContext.Provider value={value}>{children}</ShellContext.Provider>;
}

export function useShell(): ShellContextValue {
  const value = useContext(ShellContext);
  if (!value) throw new Error("useShell must be used inside the shell");
  return value;
}
