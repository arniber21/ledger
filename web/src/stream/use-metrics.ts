import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { applyStreamEvent, parseStreamData } from "./reduce";
import { emptySnapshot, type MetricsSnapshot } from "./types";

export const metricsKey = ["sentinel", "metrics"] as const;

/**
 * Reads `GET /v1/verdicts/stream`, or `VITE_SENTINEL_SSE` when that is set.
 * A stream that is down leaves the snapshot empty. Nothing is invented.
 */
export function useMetrics(): MetricsSnapshot {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: metricsKey,
    queryFn: () => emptySnapshot(),
    initialData: emptySnapshot,
    staleTime: Infinity,
    refetchOnMount: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });

  useEffect(() => {
    const url = import.meta.env.VITE_SENTINEL_SSE || "/v1/verdicts/stream";

    const source = new EventSource(url);
    const write = (next: MetricsSnapshot) => {
      queryClient.setQueryData(metricsKey, next);
    };

    source.onopen = () => {
      const current = queryClient.getQueryData<MetricsSnapshot>(metricsKey) ?? emptySnapshot();
      write({ ...current, connection: "live" });
    };

    source.onerror = () => {
      const current = queryClient.getQueryData<MetricsSnapshot>(metricsKey) ?? emptySnapshot();
      write({ ...current, connection: "offline" });
    };

    source.onmessage = (message) => {
      const event = parseStreamData(message.data);
      if (!event) return;
      const current = queryClient.getQueryData<MetricsSnapshot>(metricsKey) ?? emptySnapshot();
      write(applyStreamEvent(current, event));
    };

    return () => {
      source.close();
    };
  }, [queryClient]);

  return query.data;
}
