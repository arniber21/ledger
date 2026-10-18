import { useEffect, useState, useSyncExternalStore } from "react";
import { getShortcuts, subscribeShortcuts, useHotkeys } from "./hotkeys";

function pretty(keys: string): string {
  return keys
    .split(" ")
    .map((step) =>
      step
        .split("+")
        .map((p) => (p === "mod" ? "⌘" : p === "shift" ? "⇧" : p === "escape" ? "Esc" : p === "enter" ? "↵" : p.length === 1 ? p.toUpperCase() : p))
        .join(" "),
    )
    .join(" then ");
}

export function ShortcutsHelp() {
  const [open, setOpen] = useState(false);
  const shortcuts = useSyncExternalStore(subscribeShortcuts, getShortcuts, getShortcuts);
  useHotkeys({ "?": { handler: () => setOpen((v) => !v), hidden: true }, "!escape": { handler: () => setOpen(false), hidden: true } }, { scope: "help", enabled: true });
  useEffect(() => {
    const onOpen = () => setOpen(true);
    window.addEventListener("ledger:shortcuts", onOpen);
    return () => window.removeEventListener("ledger:shortcuts", onOpen);
  }, []);
  if (!open) return null;
  const scopes = [...new Set(shortcuts.map((s) => s.scope))];
  return (
    <div className="scrim" onClick={() => setOpen(false)}>
      <div className="sheet shortcuts-help" role="dialog" aria-modal="true" aria-labelledby="shortcuts-title" data-testid="shortcuts" onClick={(e) => e.stopPropagation()}>
        <div className="sheet-head">
          <h2 id="shortcuts-title">Shortcuts</h2>
          <button type="button" onClick={() => setOpen(false)}>Esc</button>
        </div>
        {scopes.map((scope) => (
          <section key={scope}>
            <h3>{scope}</h3>
            <ul>
              {shortcuts.filter((s) => s.scope === scope).map((s) => (
                <li key={s.keys}>
                  <span>{s.description}</span>
                  <kbd>{pretty(s.keys)}</kbd>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>
    </div>
  );
}

export function openShortcutsHelp() {
  window.dispatchEvent(new Event("ledger:shortcuts"));
}
