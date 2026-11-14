import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { create } from "zustand";

import {
  commands,
  type CullOutcome,
  type CullingAction,
  type LibraryFilter_Deserialize,
  type LibraryKind,
  type LibraryOrigin,
  type LibraryRequest_Deserialize,
} from "../../bindings";
import { useCommand } from "../../lib/ipc";
import { queryKeys } from "../../lib/queryKeys";

/**
 * The Library's data: the grid's snapshot and a result's page, computed in Rust and asked again on `LibraryChanged`,
 * plus the view state that outlives the grid while a result page is open.
 */

/** The grid as Rust computes it for `request`; the last answer stays while the next one loads. */
export function useLibrarySnapshot(request: LibraryRequest_Deserialize) {
  return useQuery({
    queryKey: queryKeys.library.snapshot(request),
    queryFn: () => commands.librarySnapshot(request),
    placeholderData: keepPreviousData,
  });
}

/** A video card's still frame, taken once by Rust. */
export function useThumbnail(id: string, isEnabled: boolean) {
  return useQuery({
    queryKey: queryKeys.library.thumbnail(id),
    queryFn: () => commands.libraryThumbnail(id),
    enabled: isEnabled,
    staleTime: Infinity,
  });
}

export function useResultDetail(id: string) {
  return useCommand(queryKeys.library.detail(id), () => commands.libraryDetail(id));
}

export function useImageDetails(id: string, versionId: string | null) {
  return useCommand(queryKeys.library.image(id, versionId), () => commands.libraryImageDetails(id, versionId));
}

export type Filter = Required<LibraryFilter_Deserialize>;

export const defaultFilter: Filter = {
  query: "",
  mode: null,
  model: null,
  resolution: null,
  finish: "all",
  imageModel: null,
  imageAspectRatio: null,
  isStarredOnly: false,
  showsRejected: false,
  sort: "newest",
};

interface LibraryViewState {
  origin: LibraryOrigin;
  kind: LibraryKind;
  filter: Filter;
  expandedGroups: string[];
  /** The video results in grid order, for the result page's previous and next. */
  order: string[];
  set: (change: Partial<Omit<LibraryViewState, "set">>) => void;
}

/** What the grid shows, kept while a result page is open so Back returns to the same place. */
export const useLibraryView = create<LibraryViewState>((set) => ({
  origin: "local",
  kind: "all",
  filter: defaultFilter,
  expandedGroups: [],
  order: [],
  set: (change) => {
    set(change);
  },
}));

/**
 * Keep, reject or clear `ids`. `cursor` is the current card when it alone is culled: Rust says where it goes next in
 * `order` (`LibraryCursor.advanced`); `showsRejected` keeps a rejected card on screen.
 */
export function cull(
  action: CullingAction,
  ids: string[],
  cursor: { id: string; order: string[]; showsRejected: boolean } | null = null,
): Promise<CullOutcome> {
  if (ids.length === 0) return Promise.resolve({ action: null, cursor: cursor?.id ?? null });
  return commands.libraryCull(action, ids, cursor?.id ?? null, cursor?.order ?? [], cursor?.showsRejected ?? false);
}
