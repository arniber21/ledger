import { useQuery, type QueryClient } from "@tanstack/react-query";
import type { SpecRun } from "./types";

export const specsKey = ["specs", "runs"] as const;

/** Built-in spec as `GET /api/specs` sends it. */
export type BuiltinSpec = {
  id: string;
  pattern: string;
  scope: string;
  policy: string;
};

export function parseBuiltins(body: unknown): BuiltinSpec[] {
  if (typeof body !== "object" || body === null || !("specs" in body) || !Array.isArray(body.specs)) return [];
  const specs: BuiltinSpec[] = [];
  for (const item of body.specs) {
    if (typeof item !== "object" || item === null) continue;
    const spec = item as Record<string, unknown>;
    if (spec.status !== "armed") continue;
    if (typeof spec.id !== "string" || spec.id.length === 0) continue;
    specs.push({
      id: spec.id,
      pattern: typeof spec.pattern === "string" ? spec.pattern : "",
      scope: typeof spec.scope === "string" ? spec.scope : "",
      policy: typeof spec.policy === "string" ? spec.policy : "",
    });
  }
  return specs;
}

/** An armed built-in spec shown as a read-only run. */
export function builtinRun(spec: BuiltinSpec): SpecRun {
  return {
    key: spec.id,
    specId: spec.id,
    sentence: spec.policy,
    clauses: null,
    patternId: spec.id.split(".").slice(1).join("."),
    parameters: spec.scope ? { binding_attribute: spec.scope } : {},
    status: "armed",
    match: null,
    rejectReason: null,
    monitor: "running",
    draftedBy: "Built in",
    steps: [
      { kind: "policy", text: spec.policy },
      { kind: "monitor", patternId: spec.pattern, monitor: "Running on the live stream" },
    ],
    error: null,
  };
}

/** Built-in specs the API reports as armed. A failed read lists none. */
export function useBuiltinSpecs(): SpecRun[] {
  const query = useQuery({
    queryKey: ["specs", "builtin"],
    queryFn: async () => {
      const response = await fetch("/api/specs");
      if (!response.ok) throw new Error("Specs are unavailable.");
      return parseBuiltins(await response.json()).map(builtinRun);
    },
    retry: false,
    staleTime: 10_000,
    refetchInterval: 15_000,
  });
  return query.data ?? [];
}

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
  const builtin = useBuiltinSpecs();
  return [...builtin, ...query.data];
}

export function insertRun(queryClient: QueryClient, run: SpecRun) {
  queryClient.setQueryData<SpecRun[]>(specsKey, (current) => [...(current ?? []), run]);
}

export function replaceRun(queryClient: QueryClient, key: string, recipe: (run: SpecRun) => SpecRun) {
  queryClient.setQueryData<SpecRun[]>(specsKey, (current) =>
    (current ?? []).map((run) => (run.key === key ? recipe(run) : run)),
  );
}
