import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { applySamples, applyStreamEvent, parseStreamData } from "./reduce";
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

    // The stream carries verdicts. Counters and series come from the store.
    let stopped = false;
    const poll = async () => {
      try {
        const response = await fetch("/api/metrics");
        if (!response.ok || stopped) return;
        const body: unknown = await response.json();
        const current = queryClient.getQueryData<MetricsSnapshot>(metricsKey) ?? emptySnapshot();
        write(applySamples(current, body));
      } catch {
        // The counters keep their last value. Nothing is invented.
      }
    };
    void poll();
    const timer = window.setInterval(() => void poll(), 5_000);

    return () => {
      stopped = true;
      window.clearInterval(timer);
      source.close();
    };
  }, [queryClient]);

  return query.data;
}
