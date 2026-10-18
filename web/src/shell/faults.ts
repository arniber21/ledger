import { useCallback, useRef, useState } from "react";
import { useToast } from "../ui";

export const FAULTS = ["double-pay", "no-approval", "exceed-budget", "swapped-vendor", "hidden-memo"] as const;

export type FaultName = (typeof FAULTS)[number];

export type FaultOutcome = { ok: true; enabled: boolean } | { ok: false; reason: string };

/** Reads the `{ enabled }` body of `POST /api/faults/:name`. Anything else is not a success. */
export function parseFaultBody(body: unknown): boolean | null {
  if (typeof body === "object" && body !== null && "enabled" in body && typeof body.enabled === "boolean") {
    return body.enabled;
  }
  return null;
}

/** Plain-language failure line. The desk runs a full cycle per toggle, so a timeout is common. */
export function faultFailure(name: string, reason: string): string {
  return `Could not toggle ${name}. ${reason}`;
}

export async function postFault(name: FaultName, signal?: AbortSignal): Promise<FaultOutcome> {
  try {
    const response = await fetch(`/api/faults/${name}`, { method: "POST", signal });
    if (!response.ok) return { ok: false, reason: "Fault control is not connected." };
    const enabled = parseFaultBody(await response.json().catch(() => null));
    if (enabled === null) return { ok: false, reason: "The desk did not report the fault state." };
    return { ok: true, enabled };
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") return { ok: false, reason: "Cancelled." };
    return { ok: false, reason: "The desk did not answer." };
  }
}

/**
 * One fault toggle at a time. The toggle can take tens of seconds because the desk
 * runs a full cycle, so a pending toast stays up and a second submit is ignored.
 * Undo posts the same toggle again and only claims success when the desk says so.
 */
export function useFaultControl(onState?: (name: FaultName, enabled: boolean) => void) {
  const toast = useToast();
  const busy = useRef(false);
  const [pending, setPending] = useState<FaultName | null>(null);

  const toggle = useCallback(
    async (name: FaultName, intent: "on" | "toggle" = "toggle"): Promise<boolean> => {
      if (busy.current) return false;
      busy.current = true;
      setPending(name);
      const waiting = toast.push({
        message: `Toggling ${name}… the desk is running a full cycle, this can take a while.`,
        tone: "info",
        durationMs: 120_000,
      });
      try {
        const result = await postFault(name);
        toast.dismiss(waiting);
        if (!result.ok) {
          toast.push({ message: faultFailure(name, result.reason), tone: "error" });
          return false;
        }
        onState?.(name, result.enabled);
        toast.push({
          message: result.enabled
            ? `${name} is on. A payment that carries it needs review.`
            : `${name} is off.`,
          tone: "success",
          undo: async () => {
            if (busy.current) return;
            busy.current = true;
            setPending(name);
            try {
              const back = await postFault(name);
              if (!back.ok) {
                toast.push({ message: faultFailure(name, back.reason), tone: "error" });
                return;
              }
              onState?.(name, back.enabled);
              toast.push({ message: back.enabled ? `${name} is on.` : `${name} is off.`, tone: "info" });
            } finally {
              busy.current = false;
              setPending(null);
            }
          },
        });
        return intent === "on" ? result.enabled : true;
      } finally {
        busy.current = false;
        setPending(null);
      }
    },
    [toast, onState],
  );

  return { toggle, pending };
}
