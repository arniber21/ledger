import { useQuery, type QueryClient } from "@tanstack/react-query";
import type { SpecRun } from "./types";

export const specsKey = ["specs", "runs"] as const;

export function useSpecRuns(): SpecRun[] {
  const query = useQuery({
    queryKey: specsKey,
    queryFn: () => [] as SpecRun[],
    initialData: [] as SpecRun[],
    staleTime: Infinity,
    refetchOnMount: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
  return query.data;
}

export function insertRun(queryClient: QueryClient, run: SpecRun) {
  queryClient.setQueryData<SpecRun[]>(specsKey, (current) => [...(current ?? []), run]);
}

export function replaceRun(queryClient: QueryClient, key: string, recipe: (run: SpecRun) => SpecRun) {
  queryClient.setQueryData<SpecRun[]>(specsKey, (current) =>
    (current ?? []).map((run) => (run.key === key ? recipe(run) : run)),
  );
}
