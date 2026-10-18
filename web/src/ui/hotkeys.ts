import { useEffect, useRef } from "react";

// Keyboard core. Map keys look like "j", "shift+j", "mod+a", "g i" (sequence), "?", "escape".
// Keystrokes in inputs, textareas and contenteditable are ignored unless the binding is
// prefixed with "!" (for example "!escape"), which is allowed while typing.

export type HotkeyHandler = (event: KeyboardEvent) => void;
export type HotkeyEntry = HotkeyHandler | { handler: HotkeyHandler; description?: string; hidden?: boolean };
export type HotkeyMap = Record<string, HotkeyEntry>;
export type HotkeyOptions = { scope?: string; enabled?: boolean; sequenceMs?: number };

export type RegisteredShortcut = { keys: string; description: string; scope: string };

const registry = new Map<symbol, RegisteredShortcut[]>();
const listeners = new Set<() => void>();
let snapshot: readonly RegisteredShortcut[] = [];

function publish() {
  snapshot = [...registry.values()].flat();
  for (const listener of listeners) listener();
}

export function subscribeShortcuts(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getShortcuts(): readonly RegisteredShortcut[] {
  return snapshot;
}

export function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target.isContentEditable;
}

type Step = { key: string; mod: boolean; shift: boolean; alt: boolean };

export function parseCombo(combo: string): Step[] {
  return combo
    .replace(/^!/, "")
    .trim()
    .split(/\s+/)
    .map((part) => {
      const pieces = part.toLowerCase().split("+");
      let key = pieces[pieces.length - 1] ?? "";
      if (part === "+" ) key = "+";
      const flags = pieces.slice(0, -1);
      return {
        key,
        mod: flags.includes("mod") || flags.includes("cmd") || flags.includes("ctrl"),
        shift: flags.includes("shift"),
        alt: flags.includes("alt"),
      };
    });
}

export function stepMatches(step: Step, event: KeyboardEvent): boolean {
  const mod = event.metaKey || event.ctrlKey;
  if (step.mod !== mod) return false;
  if (step.alt !== event.altKey) return false;
  const key = event.key.toLowerCase();
  const wanted = step.key === "esc" ? "escape" : step.key;
  if (key !== wanted) return false;
  // Printable characters already encode shift ("?" , "J"), so only enforce it for named combos.
  if (step.shift) return event.shiftKey;
  if (wanted.length > 1 || mod) return !event.shiftKey;
  return !(event.key.length === 1 && event.key !== event.key.toLowerCase() && /[a-z]/i.test(event.key));
}

type Compiled = { combo: string; steps: Step[]; handler: HotkeyHandler; allowTyping: boolean };

export function useHotkeys(map: HotkeyMap, options: HotkeyOptions = {}): void {
  const mapRef = useRef(map);
  mapRef.current = map;
  const { scope = "Global", enabled = true, sequenceMs = 1200 } = options;
  const signature = Object.keys(map).join("|") +
    Object.values(map).map((entry) => (typeof entry === "function" ? "" : `${entry.description}${entry.hidden}`)).join("|");

  useEffect(() => {
    if (!enabled) return;
    const id = Symbol(scope);
    const build = (): Compiled[] =>
      Object.entries(mapRef.current).map(([combo, entry]) => ({
        combo,
        steps: parseCombo(combo),
        allowTyping: combo.startsWith("!"),
        handler: (event) => (typeof entry === "function" ? entry : entry.handler)(event),
      }));
    registry.set(
      id,
      Object.entries(mapRef.current).flatMap(([combo, entry]) =>
        typeof entry === "function" || entry.hidden || !entry.description
          ? []
          : [{ keys: combo.replace(/^!/, ""), description: entry.description, scope }],
      ),
    );
    publish();

    let buffer: Step[] = [];
    let timer = 0;
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing) return;
      if (["Shift", "Meta", "Control", "Alt"].includes(event.key)) return;
      const typing = isTypingTarget(event.target);
      const compiled = build();
      const pending = buffer.length;
      const candidates = compiled.filter((item) => (typing ? item.allowTyping : true));

      if (pending > 0) {
        const prefix = buffer;
        for (const item of candidates) {
          if (item.steps.length !== pending + 1) continue;
          const ok =
            prefix.every((step, i) => item.steps[i]?.key === step.key) &&
            stepMatches(item.steps[pending] as Step, event);
          if (ok) {
            buffer = [];
            window.clearTimeout(timer);
            event.preventDefault();
            item.handler(event);
            return;
          }
        }
        buffer = [];
        window.clearTimeout(timer);
      }

      for (const item of candidates) {
        if (item.steps.length === 1 && stepMatches(item.steps[0] as Step, event)) {
          event.preventDefault();
          item.handler(event);
          return;
        }
      }
      const starts = candidates.some(
        (item) => item.steps.length > 1 && stepMatches(item.steps[0] as Step, event),
      );
      if (starts) {
        buffer = [{ key: event.key.toLowerCase(), mod: false, shift: false, alt: false }];
        window.clearTimeout(timer);
        timer = window.setTimeout(() => {
          buffer = [];
        }, sequenceMs);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.clearTimeout(timer);
      registry.delete(id);
      publish();
    };
  }, [enabled, scope, sequenceMs, signature]);
}
