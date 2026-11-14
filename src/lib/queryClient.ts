import { QueryClient } from "@tanstack/react-query";

/**
 * The query cache for command results. Rust tells the window when its state changes (`useInvalidateOn`), so a
 * result stays fresh until an event says otherwise: no refetch on focus or on a timer, and no retry of a command
 * that failed, since a failure is an answer (`AppError`), not a dropped connection.
 */
export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { staleTime: Infinity, retry: false, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
}

export const queryClient = createQueryClient();
