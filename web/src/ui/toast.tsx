import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { isTypingTarget } from "./hotkeys";

export type ToastTone = "success" | "info" | "error";
export type ToastInput = {
  message: string;
  tone?: ToastTone;
  undo?: () => void | Promise<void>;
  durationMs?: number;
};
type ToastItem = ToastInput & { id: number; tone: ToastTone; leaving: boolean };
type ToastApi = { push: (toast: ToastInput) => number; dismiss: (id: number) => void; undoLatest: () => boolean };

const ToastContext = createContext<ToastApi | null>(null);
const FALLBACK: ToastApi = { push: () => 0, dismiss: () => {}, undoLatest: () => false };

export function useToast(): ToastApi {
  return useContext(ToastContext) ?? FALLBACK;
}

const EXIT_MS = 160;
const MAX_VISIBLE = 4;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const nextId = useRef(1);
  const itemsRef = useRef(items);
  itemsRef.current = items;

  const dismiss = useCallback((id: number) => {
    setItems((current) => current.map((item) => (item.id === id ? { ...item, leaving: true } : item)));
    window.setTimeout(() => setItems((current) => current.filter((item) => item.id !== id)), EXIT_MS);
  }, []);

  const push = useCallback((toast: ToastInput) => {
    const id = nextId.current++;
    setItems((current) => [...current, { ...toast, id, tone: toast.tone ?? "info", leaving: false }].slice(-MAX_VISIBLE));
    return id;
  }, []);

  const undoLatest = useCallback(() => {
    const latest = [...itemsRef.current].reverse().find((item) => item.undo && !item.leaving);
    if (!latest?.undo) return false;
    dismiss(latest.id);
    void Promise.resolve(latest.undo());
    return true;
  }, [dismiss]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (isTypingTarget(event.target)) return;
      const cmdZ = (event.metaKey || event.ctrlKey) && !event.shiftKey && event.key.toLowerCase() === "z";
      const plainZ = !event.metaKey && !event.ctrlKey && !event.altKey && event.key === "z";
      if ((cmdZ || plainZ) && undoLatest()) event.preventDefault();
      if (event.key === "Escape" && itemsRef.current.some((item) => !item.leaving)) {
        const last = [...itemsRef.current].reverse().find((item) => !item.leaving);
        if (last && document.querySelector("[data-toast-host]:hover, [data-toast-host]:focus-within")) {
          event.preventDefault();
          dismiss(last.id);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [dismiss, undoLatest]);

  const api = useMemo(() => ({ push, dismiss, undoLatest }), [push, dismiss, undoLatest]);

  return (
    <ToastContext.Provider value={api}>
      {children}
      <ToastHost items={items} dismiss={dismiss} undoLatest={undoLatest} />
    </ToastContext.Provider>
  );
}

function ToastHost({ items, dismiss }: { items: ToastItem[]; dismiss: (id: number) => void; undoLatest: () => boolean }) {
  return (
    <div className="toast-host" data-toast-host data-testid="toast-host" role="region" aria-label="Notifications">
      <div className="toast-live" aria-live="polite" aria-atomic="false">
        {items.map((item) => (
          <ToastView key={item.id} item={item} dismiss={dismiss} />
        ))}
      </div>
    </div>
  );
}

function ToastView({ item, dismiss }: { item: ToastItem; dismiss: (id: number) => void }) {
  const [paused, setPaused] = useState(false);
  const remaining = useRef(item.durationMs ?? (item.undo ? 6000 : item.tone === "error" ? 8000 : 4000));
  const startedAt = useRef(0);
  const dragStart = useRef<number | null>(null);
  const [dx, setDx] = useState(0);

  useEffect(() => {
    if (paused || item.leaving) return;
    startedAt.current = Date.now();
    const timer = window.setTimeout(() => dismiss(item.id), remaining.current);
    return () => {
      window.clearTimeout(timer);
      remaining.current = Math.max(1000, remaining.current - (Date.now() - startedAt.current));
    };
  }, [paused, item.leaving, item.id, dismiss]);

  return (
    <div
      className="toast"
      data-tone={item.tone}
      data-leaving={item.leaving || undefined}
      data-testid="toast"
      role={item.tone === "error" ? "alert" : "status"}
      style={dx ? { transform: `translateX(${dx}px)`, opacity: Math.max(0.2, 1 - Math.abs(dx) / 240), transition: "none" } : undefined}
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
      onFocus={() => setPaused(true)}
      onBlur={() => setPaused(false)}
      onPointerDown={(event) => {
        if ((event.target as HTMLElement).closest("button")) return;
        dragStart.current = event.clientX;
      }}
      onPointerMove={(event) => {
        if (dragStart.current !== null) setDx(event.clientX - dragStart.current);
      }}
      onPointerUp={() => {
        if (Math.abs(dx) > 80) dismiss(item.id);
        dragStart.current = null;
        setDx(0);
      }}
    >
      <span className="toast-message">{item.message}</span>
      {item.undo ? (
        <button
          type="button"
          className="toast-action"
          onClick={() => {
            dismiss(item.id);
            void Promise.resolve(item.undo?.());
          }}
        >
          Undo <kbd>Z</kbd>
        </button>
      ) : null}
      <button type="button" className="toast-close" aria-label="Dismiss" onClick={() => dismiss(item.id)}>
        ×
      </button>
    </div>
  );
}
