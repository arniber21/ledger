import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { isUuid, parseTriagePayload, type Triage, type TriageStatus } from "./model";

export const triageKey = ["inbox", "triage"] as const;

export type TriagePatch = {
  verdictId: string;
  status: TriageStatus;
  assignee: string | null;
};

async function fetchTriage(): Promise<Triage[]> {
  const response = await fetch("/api/inbox/triage");
  if (!response.ok) throw new Error("Triage status is unavailable.");
  return parseTriagePayload(await response.json());
}

function upsert(rows: readonly Triage[], patch: TriagePatch): Triage[] {
  const next = rows.filter((row) => row.verdictId !== patch.verdictId);
  next.unshift({
    verdictId: patch.verdictId,
    status: patch.status,
    assignee: patch.assignee,
    updatedAt: new Date().toISOString(),
    updatedBy: "me",
  });
  return next;
}

export function useTriage(): Triage[] {
  const query = useQuery({
    queryKey: triageKey,
    queryFn: fetchTriage,
    retry: false,
    staleTime: 5_000,
  });
  return query.data ?? [];
}

export function useSetTriage() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (patch: TriagePatch) => {
      if (!isUuid(patch.verdictId)) {
        throw new Error("This verdict is not in the store, so its status was not saved.");
      }
      const response = await fetch("/api/inbox/triage", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          verdictId: patch.verdictId,
          status: patch.status,
          assignee: patch.assignee,
        }),
      });
      if (response.status === 404) {
        throw new Error("That verdict is not in the store, so its status was not saved.");
      }
      if (!response.ok) throw new Error("Status was not saved.");
      return parseTriagePayload({ items: [await response.json()] });
    },
    onMutate: async (patch) => {
      await queryClient.cancelQueries({ queryKey: triageKey });
      const previous = queryClient.getQueryData<Triage[]>(triageKey) ?? [];
      queryClient.setQueryData(triageKey, upsert(previous, patch));
      return { previous };
    },
    onError: (_error, _patch, context) => {
      if (context) queryClient.setQueryData(triageKey, context.previous);
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: triageKey });
    },
  });
}
