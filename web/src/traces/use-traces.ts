import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { applyTracesEvent, emptyTraces, parseEventList, parseTracesData, type TracesSnapshot } from "./model";

export const tracesKey = ["sentinel", "traces"] as const;

/**
 * Loads stored events, then appends new rows from the trace SSE stream.
 * A missing API or an empty hypertable stays empty. Nothing is invented.
 */
export function useTraces(): TracesSnapshot {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: tracesKey,
    queryFn: async () => {
      const loaded = await loadTraces();
      const current = queryClient.getQueryData<TracesSnapshot>(tracesKey) ?? emptyTraces();
      if (loaded.connection === "offline") return current.events.length > 0 ? current : loaded;
      return applyTracesEvent(current, { kind: "hello", events: loaded.events });
    },
    initialData: emptyTraces,
    staleTime: Infinity,
    refetchOnMount: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });

  useEffect(() => {
    const source = new EventSource("/api/traces/stream");
    const write = (next: TracesSnapshot) => {
      queryClient.setQueryData(tracesKey, next);
    };

    source.onmessage = (message) => {
      const event = parseTracesData(message.data);
      if (!event) return;
      const current = queryClient.getQueryData<TracesSnapshot>(tracesKey) ?? emptyTraces();
      write(applyTracesEvent(current, event));
    };

    source.onerror = () => {
      const current = queryClient.getQueryData<TracesSnapshot>(tracesKey) ?? emptyTraces();
      write({ ...current, connection: "offline" });
    };

    return () => {
      source.close();
    };
  }, [queryClient]);

  return query.data;
}

async function loadTraces(): Promise<TracesSnapshot> {
  try {
    const response = await fetch("/api/traces");
    if (!response.ok) return emptyTraces();
    const body: unknown = await response.json();
    if (typeof body !== "object" || body === null || !("events" in body)) return emptyTraces();
    return { connection: "live", events: parseEventList(body.events) };
  } catch {
    return emptyTraces();
  }
}
