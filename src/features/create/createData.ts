import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { create } from "zustand";

import {
  commands,
  events,
  type AppError,
  type CreateEdit_Deserialize,
  type CreateSnapshot_Serialize,
  type LoadOutcome_Serialize,
  type Notice,
  type QueueConfirmation,
  type SubmitOutcome,
} from "../../bindings";
import { useToastStore } from "../../app/toastStore";
import { useInvalidateOn } from "../../lib/events";
import { queryKeys } from "../../lib/queryKeys";

/**
 * Create's data: the snapshot Rust computes from the form (`create_snapshot`), replaced by the answer of every edit and
 * asked again on `CreateChanged` (a quote came back, the proxy progressed). No rule of the form lives here.
 */

/** The form as Create shows it; `null` or `undefined` until Rust answers. */
export function useCreateSnapshot() {
  useInvalidateOn(events.createChanged, [queryKeys.create.snapshot]);
  return useQuery<CreateSnapshot_Serialize | null, AppError>({
    queryKey: queryKeys.create.snapshot,
    queryFn: () => commands.createSnapshot(),
  });
}

/** Sends one edit; the form Rust answers with replaces the cached one. A failure is shown as a notice. */
export function useCreateEdit(): (edit: CreateEdit_Deserialize) => void {
  const client = useQueryClient();
  return useCallback(
    (edit) => {
      commands.createEdit(edit).then(
        (snapshot) => {
          client.setQueryData(queryKeys.create.snapshot, snapshot);
        },
        (error: unknown) => {
          showError("Can't change the form", error);
        },
      );
    },
    [client],
  );
}

/** A file served to the window by `media_url`; `undefined` until known, `null` without a path. */
export function useMediaUrl(path: string | null | undefined): string | null | undefined {
  return useQuery({
    queryKey: [...queryKeys.create.all, "media", path ?? null],
    queryFn: () => (path ? commands.mediaUrl(path) : null),
    enabled: Boolean(path),
    staleTime: Infinity,
  }).data;
}

interface NoticeState {
  /** An alert Create shows over itself: a file that couldn't be opened, a job whose source is gone. */
  notice: Notice | null;
  /** Generate or Render final asks first: over today's cap, or the price isn't known under one. */
  confirmation: { question: QueueConfirmation; approve: () => void } | null;
  show: (notice: Notice) => void;
  ask: (question: QueueConfirmation, approve: () => void) => void;
  dismiss: () => void;
}

export const useCreateNotice = create<NoticeState>((set) => ({
  notice: null,
  confirmation: null,
  show: (notice) => {
    set({ notice });
  },
  ask: (question, approve) => {
    set({ confirmation: { question, approve } });
  },
  dismiss: () => {
    set({ notice: null, confirmation: null });
  },
}));

/** A failed command's message. */
export function errorText(error: unknown): string {
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
  ) {
    return error.message;
  }
  return String(error);
}

/** Opens the file picker for the source video (Add a video, the empty stage, Ctrl+O). */
export function chooseSource(): void {
  commands.createChooseSource().catch((error: unknown) => {
    showError("Can't open the video", error);
  });
}

/** Shows a failed command's message under `title`. */
export function showError(title: string, error: unknown): void {
  useCreateNotice.getState().show({ title, detail: errorText(error) });
}

/** What a submit answered: a toast when queued, the question when it needs one, the blocker as a notice. */
export function handleSubmitOutcome(outcome: SubmitOutcome, again: () => void): void {
  switch (outcome.kind) {
    case "queued":
      useToastStore.getState().showQueued(outcome.notice);
      return;
    case "confirm":
      useCreateNotice.getState().ask(outcome.confirmation, again);
      return;
    case "blocked":
      useCreateNotice.getState().show({ title: "Can't queue this yet", detail: outcome.reason });
      return;
    case "busy":
    case "failed":
      // Busy: the first press is still being queued. Failed: the connection alert says why.
      return;
  }
}

/** Generate or Draft: asks again approved once the person confirms the price. */
export async function submit(isDraft: boolean, approved = false): Promise<void> {
  try {
    const outcome = await commands.createSubmit(isDraft, approved);
    handleSubmitOutcome(outcome, () => void submit(isDraft, true));
  } catch (error) {
    showError("Can't queue the job", error);
  }
}

/** Render final from a draft: queued at once, with the same price confirmation. */
export async function renderFinal(id: string, approved = false): Promise<void> {
  try {
    const outcome = await commands.createRenderFinal(id, approved);
    handleSubmitOutcome(outcome, () => void renderFinal(id, true));
  } catch (error) {
    showError("Can't queue the final", error);
  }
}

/**
 * Edit Inputs and Generate again: the job's inputs replace the form; a draft that lost inputs is offered back with
 * Undo. Resolves once the form is loaded, so the caller can open Create.
 */
export async function loadJob(id: string): Promise<void> {
  try {
    showLoaded(await commands.createLoadJob(id));
  } catch (error) {
    showError("Can't open the job in Create", error);
  }
}

/** Edit… and Extend… on a result: it opens as the source of a Seedance job made from it, as `loadJob` loads one. */
export async function openFollowUp(id: string, mode: "edit" | "extend"): Promise<void> {
  try {
    showLoaded(await commands.createOpenFollowUp(id, mode));
  } catch (error) {
    showError("Can't open the job in Create", error);
  }
}

function showLoaded(outcome: LoadOutcome_Serialize) {
  if (outcome.canUndo) {
    useToastStore.getState().showDraftReplaced(() => void commands.createUndoReplacement());
  }
  if (outcome.notice) useCreateNotice.getState().show(outcome.notice);
}
