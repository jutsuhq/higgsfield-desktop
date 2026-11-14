import { useQuery } from "@tanstack/react-query";
import { Channel } from "@tauri-apps/api/core";
import { useEffect } from "react";
import { create } from "zustand";

import { commands, type JobProgress, type QueueSnapshot_Serialize } from "../../bindings";
import { queryKeys } from "../../lib/queryKeys";

/**
 * The Queue's data: the snapshot Rust computes (rows, sections, counts), asked again on `QueueChanged` and on a timer
 * so the elapsed times and relative times move (every second while jobs run, as the Swift rows do, else every 30 s),
 * and the step progress Rust streams through a channel.
 */

/** Every second while a job is in the queue, as `QueueView` redraws; every 30 s otherwise. */
export function refreshInterval(snapshot: QueueSnapshot_Serialize | null | undefined): number {
  return snapshot && snapshot.activeCount > 0 ? 1_000 : 30_000;
}

/** The Queue at this moment; `undefined` until the first answer. */
export function useQueueSnapshot() {
  return useQuery({
    queryKey: queryKeys.queue.snapshot,
    queryFn: async () => {
      const snapshot = await commands.queueSnapshot();
      // The snapshot carries each job's latest progress; older streamed values would hold it back.
      useQueueProgress.getState().clear();
      return snapshot;
    },
    refetchInterval: (query) => refreshInterval(query.state.data),
  });
}

interface ProgressState {
  /** The step progress of each job, 0…1, since the last snapshot. */
  fractions: Record<string, number>;
  set: (progress: JobProgress) => void;
  clear: () => void;
}

/** Progress streamed from Rust between snapshots. */
export const useQueueProgress = create<ProgressState>((set) => ({
  fractions: {},
  set: ({ jobId, fraction }) => {
    set((state) => ({ fractions: { ...state.fractions, [jobId]: fraction ?? 0 } }));
  },
  clear: () => {
    set({ fractions: {} });
  },
}));

/** Opens the progress channel while the Queue is on screen; Rust drops the previous one when a new one opens. */
export function useProgressChannel(): void {
  useEffect(() => {
    const channel = new Channel<JobProgress>();
    channel.onmessage = (progress) => {
      useQueueProgress.getState().set(progress);
    };
    void commands.queueProgress(channel);
    return () => {
      channel.onmessage = () => undefined;
    };
  }, []);
}

/** The progress to draw for job `id`: the streamed value when there is one, else the snapshot's. */
export function useJobProgress(id: string, fromSnapshot: number | null): number | null {
  const streamed = useQueueProgress((state) => state.fractions[id]);
  if (fromSnapshot === null) return null;
  return streamed ?? fromSnapshot;
}
