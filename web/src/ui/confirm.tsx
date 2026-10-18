import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

export type ConfirmOptions = {
  title: string;
  body?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  tone?: "default" | "danger";
};
type Pending = ConfirmOptions & { resolve: (ok: boolean) => void };

const ConfirmContext = createContext<((options: ConfirmOptions) => Promise<boolean>) | null>(null);

export function useConfirm(): (options: ConfirmOptions) => Promise<boolean> {
  return useContext(ConfirmContext) ?? (() => Promise.resolve(false));
}

export function ConfirmProvider({ children }: { children: ReactNode }) {
  const [pending, setPending] = useState<Pending | null>(null);
  const trigger = useRef<HTMLElement | null>(null);

  const confirm = useCallback((options: ConfirmOptions) => {
    trigger.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    return new Promise<boolean>((resolve) => setPending({ ...options, resolve }));
  }, []);

  const settle = useCallback(
    (ok: boolean) => {
      setPending((current) => {
        current?.resolve(ok);
        return null;
      });
      const target = trigger.current;
      window.setTimeout(() => target?.isConnected && target.focus(), 0);
    },
    [],
  );

  const value = useMemo(() => confirm, [confirm]);
  return (
    <ConfirmContext.Provider value={value}>
      {children}
      {pending ? <ConfirmDialog pending={pending} settle={settle} /> : null}
    </ConfirmContext.Provider>
  );
}

function ConfirmDialog({ pending, settle }: { pending: Pending; settle: (ok: boolean) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    confirmRef.current?.focus();
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        settle(false);
      } else if (event.key === "Enter" && !(document.activeElement instanceof HTMLButtonElement && document.activeElement.dataset.role === "cancel")) {
        event.preventDefault();
        event.stopPropagation();
        settle(true);
      } else if (event.key === "Tab") {
        const nodes = ref.current?.querySelectorAll<HTMLElement>("button");
        if (!nodes || nodes.length === 0) return;
        const first = nodes[0] as HTMLElement;
        const last = nodes[nodes.length - 1] as HTMLElement;
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      } else {
        event.stopPropagation();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [settle]);

  return (
    <div className="scrim" onClick={() => settle(false)}>
      <div
        ref={ref}
        className="sheet confirm"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        aria-describedby={pending.body ? "confirm-body" : undefined}
        data-testid="confirm"
        onClick={(event) => event.stopPropagation()}
      >
        <h2 id="confirm-title">{pending.title}</h2>
        {pending.body ? <p id="confirm-body">{pending.body}</p> : null}
        <div className="confirm-actions">
          <button type="button" data-role="cancel" onClick={() => settle(false)}>
            {pending.cancelLabel ?? "Cancel"} <kbd>Esc</kbd>
          </button>
          <button
            ref={confirmRef}
            type="button"
            className="confirm-primary"
            data-tone={pending.tone ?? "default"}
            onClick={() => settle(true)}
          >
            {pending.confirmLabel ?? "Confirm"} <kbd>↵</kbd>
          </button>
        </div>
      </div>
    </div>
  );
}
