import { useMutation, useQueryClient, type QueryKey } from "@tanstack/react-query";
import { useToast } from "./toast";

// Optimistic mutation: apply the patch to the cache immediately, roll back and toast on
// error, invalidate on settle.
export type OptimisticOptions<TVars, TData, TCache> = {
  queryKey: QueryKey;
  mutationFn: (vars: TVars) => Promise<TData>;
  apply: (current: TCache | undefined, vars: TVars) => TCache;
  errorMessage?: (error: unknown, vars: TVars) => string;
  invalidate?: QueryKey[];
};

export function useOptimisticMutation<TVars, TData = unknown, TCache = unknown>(
  options: OptimisticOptions<TVars, TData, TCache>,
) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation<TData, unknown, TVars, { previous: TCache | undefined }>({
    mutationFn: options.mutationFn,
    onMutate: async (vars) => {
      await queryClient.cancelQueries({ queryKey: options.queryKey });
      const previous = queryClient.getQueryData<TCache>(options.queryKey);
      queryClient.setQueryData<TCache>(options.queryKey, options.apply(previous, vars));
      return { previous };
    },
    onError: (error, vars, context) => {
      queryClient.setQueryData(options.queryKey, context?.previous);
      toast.push({
        tone: "error",
        message: options.errorMessage?.(error, vars) ?? (error instanceof Error ? error.message : "That change did not save."),
      });
    },
    onSettled: async () => {
      await Promise.all([options.queryKey, ...(options.invalidate ?? [])].map((queryKey) => queryClient.invalidateQueries({ queryKey })));
    },
  });
}
