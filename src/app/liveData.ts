import {
  commands,
  events,
  type AccountPlate_Serialize,
  type ConnectionDetails_Serialize,
  type ConnectionSnapshot_Serialize,
  type PlanTier,
  type Preferences_Serialize,
} from "../bindings";
import { useQueueSnapshot } from "../features/queue/queueData";
import { useInvalidateOn } from "../lib/events";
import { useCommand } from "../lib/ipc";
import { queryKeys } from "../lib/queryKeys";

/**
 * What the shell and the pages show from the app's live state. The connection and the settings come from Rust and
 * follow its events; the jobs come from the Queue's snapshot; the Library returns the state of a fresh install until it
 * is wired in.
 */

export type { PlanTier };

/** `AccountPlate`: the plate above Settings in the sidebar, the plan and what is left on it. */
export type AccountPlateModel = AccountPlate_Serialize;

/** The connection as the window shows it; `undefined` until the first answer. */
export function useConnection(): ConnectionSnapshot_Serialize | undefined {
  return useCommand(queryKeys.connection.status, () => commands.connectionStatus()).data;
}

/** The Settings › Connection card. */
export function useConnectionDetails() {
  return useCommand<ConnectionDetails_Serialize>(queryKeys.connection.details, () =>
    commands.connectionDetails(),
  );
}

export function usePreferences() {
  return useCommand<Preferences_Serialize>(queryKeys.preferences, () => commands.getPreferences());
}

/** `null` without a connection. */
export function useAccountPlate(): AccountPlateModel | null {
  return useConnection()?.plate ?? null;
}

/** `false` until the connection is known. */
export function useIsConnected(): boolean {
  return useConnection()?.kind != null;
}

/**
 * Follows Rust's events for the whole window: a `ConnectionChanged` snapshot replaces the cached one and the card
 * is asked again; `PreferencesChanged` replaces the settings; `QueueChanged` asks for the Queue again. Mounted once,
 * by the root route.
 */
export function useLiveEvents(): void {
  useInvalidateOn(events.queueChanged, [queryKeys.queue.all]);
  // What Create offers (models, prices, why it can't send) is decided by the connection, which is restored a moment
  // after the window opens: without asking again, Create keeps the answer it got before there was one.
  const onConnection = [queryKeys.connection.details, queryKeys.create.all, queryKeys.images.all];
  useInvalidateOn(events.connectionChanged, onConnection, (cache, { snapshot }) => {
    cache.setQueryData(queryKeys.connection.status, snapshot);
  });
  useInvalidateOn(events.preferencesChanged, [queryKeys.settings.fromPreferences, queryKeys.queue.all], (cache, { preferences }) => {
    cache.setQueryData(queryKeys.preferences, preferences);
  });
  useInvalidateOn(events.libraryChanged, [queryKeys.library.all]);
  useInvalidateOn(events.onDeviceRunsChanged, [queryKeys.onDevice.runs, queryKeys.tray]);
  useInvalidateOn(events.subtitlesChanged, [queryKeys.subtitles.all, queryKeys.settings.subtitles, queryKeys.library.all]);
}

/** `LiveJobs` as the sidebar reads it. */
export interface LiveJobsSummary {
  /** Jobs in the queue, paused ones included: the Queue's badge. */
  activeCount: number;
}

export function useLiveJobs(): LiveJobsSummary {
  return { activeCount: useQueueSnapshot().data?.activeCount ?? 0 };
}

/** `LibraryKindCounts`. */
export interface LibraryCounts {
  all: number;
  video: number;
  image: number;
}

const emptyLibrary: LibraryCounts = { all: 0, video: 0, image: 0 };

/** This computer's Library, without rejected results; `LibraryChanged` asks again. */
export function useLibraryCounts(): LibraryCounts {
  const counts = useCommand(queryKeys.library.counts, () => commands.libraryCounts()).data;
  return counts ? { all: counts.all, video: counts.videos, image: counts.images } : emptyLibrary;
}
