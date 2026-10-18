import { useShell } from "./context";

const ROWS = [
  ["⌘K", "Quick actions"],
  ["G then D", "Dashboard"],
  ["G then T", "Traces"],
  ["G then S", "Specs"],
  ["G then V", "Verdicts"],
  ["J / K", "Move through the feed"],
  ["Enter", "Open the selected verdict"],
  ["E", "Mark the selected verdict reviewed"],
  ["1 / 2 / 3", "Board, list, or split in the inbox"],
  ["?", "This sheet"],
  ["Esc", "Close, or go back"],
];

export function Shortcuts() {
  const { shortcutsOpen, setShortcutsOpen } = useShell();
  if (!shortcutsOpen) return null;

  return (
    <div className="scrim" onClick={() => setShortcutsOpen(false)}>
      <div
        className="sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="shortcuts-title"
        data-testid="shortcuts"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="sheet-head">
          <h2 id="shortcuts-title">Shortcuts</h2>
          <button type="button" onClick={() => setShortcutsOpen(false)}>
            Esc
          </button>
        </div>
        <ul>
          {ROWS.map(([keys, label]) => (
            <li key={keys}>
              <span>{label}</span>
              <kbd>{keys}</kbd>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
