import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";

import {
  commands,
  events,
  type AppError,
  type CreateSide,
  type ImageEdit_Deserialize,
  type ImageSnapshot_Serialize,
} from "../../bindings";
import { useInvalidateOn } from "../../lib/events";
import { queryKeys } from "../../lib/queryKeys";
import { showError, useCreateNotice } from "./createData";

/**
 * The Image side of Create (`images_snapshot`): the form, its price, the requests and the results, all computed in
 * Rust. `ImagesChanged` asks for it again; an edit's answer replaces it.
 */
export function useImagesSnapshot() {
  useInvalidateOn(events.imagesChanged, [queryKeys.images.all]);
  return useQuery<ImageSnapshot_Serialize, AppError>({
    queryKey: queryKeys.images.snapshot,
    queryFn: () => commands.imagesSnapshot(),
  });
}

/** Sends one edit of the image form; Rust's answer replaces the cached side. */
export function useImagesEdit(): (edit: ImageEdit_Deserialize) => void {
  const client = useQueryClient();
  return useCallback(
    (edit) => {
      commands.imagesEdit(edit).then(
        (snapshot) => {
          client.setQueryData(queryKeys.images.snapshot, snapshot);
        },
        (error: unknown) => {
          showError("Can't change the form", error);
        },
      );
    },
    [client],
  );
}

/** The Video | Image switch: shown at once, kept by Rust in the draft. */
export function useSetCreateSide(): (side: CreateSide) => void {
  const client = useQueryClient();
  return useCallback(
    (side) => {
      client.setQueryData<ImageSnapshot_Serialize>(queryKeys.images.snapshot, (snapshot) =>
        snapshot ? { ...snapshot, side } : snapshot,
      );
      commands.imagesSetSide(side).then(
        () => client.invalidateQueries({ queryKey: queryKeys.images.all }),
        (error: unknown) => {
          showError("Can't change the form", error);
        },
      );
    },
    [client],
  );
}

/** Generate. Over today's cap, or with no price under one, it asks first and generates again approved. */
export async function generateImages(approved = false): Promise<void> {
  try {
    const outcome = await commands.imagesGenerate(approved);
    switch (outcome.kind) {
      case "confirm":
        useCreateNotice.getState().ask(outcome.confirmation, () => void generateImages(true));
        return;
      case "failed":
        useCreateNotice.getState().show(outcome.notice);
        return;
      case "blocked":
      case "started":
      case "notStarted":
        // Blocked: the button is off and the notice says why. Not started: another connection came first.
        return;
    }
  } catch (error) {
    showError("Can't generate the images", error);
  }
}
