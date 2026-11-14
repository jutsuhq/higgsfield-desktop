import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryKey,
  type UseMutationResult,
  type UseQueryResult,
} from "@tanstack/react-query";

import type { AppError } from "../bindings";

/**
 * A command's result as a query. `run` is a generated binding (`() => commands.connectionStatus()`); a failed
 * command rejects with its `AppError`.
 */
export function useCommand<T>(key: QueryKey, run: () => Promise<T>): UseQueryResult<T, AppError> {
  return useQuery<T, AppError>({ queryKey: key, queryFn: run });
}

/**
 * A command that changes something. The keys in `invalidates` are fetched again once it settles, for state no event
 * reports; state an event reports refreshes through `useInvalidateOn` instead.
 */
export function useCommandMutation<Args = void, T = unknown>(
  run: (args: Args) => Promise<T>,
  invalidates: readonly QueryKey[] = [],
): UseMutationResult<T, AppError, Args> {
  const client = useQueryClient();
  return useMutation<T, AppError, Args>({
    mutationFn: run,
    onSettled: async () => {
      await Promise.all(invalidates.map((queryKey) => client.invalidateQueries({ queryKey })));
    },
  });
}
