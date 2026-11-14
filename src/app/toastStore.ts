import { create } from "zustand";

/** `QueuedNotice`: what the last submit queued, shown for a moment over every screen. */
export interface QueuedNotice {
  id: string;
  text: string;
}

/** `DraftReplacement`: the draft the last replacement lost, offered back with Undo. */
export interface DraftReplacement {
  id: string;
  undo: () => void;
}

export const queuedNoticeDurationMs = 6_000;
export const draftReplacementDurationMs = 10_000;

interface ToastState {
  queuedNotice: QueuedNotice | null;
  draftReplacement: DraftReplacement | null;
  showQueued: (text: string) => void;
  showDraftReplaced: (undo: () => void) => void;
  dismissQueued: () => void;
  dismissDraftReplacement: () => void;
}

/** The toasts of `NavigationModel`: a queued notice replaces a draft replacement and the other way round. */
export const useToastStore = create<ToastState>((set) => ({
  queuedNotice: null,
  draftReplacement: null,
  showQueued: (text) => {
    set({ queuedNotice: { id: crypto.randomUUID(), text }, draftReplacement: null });
  },
  showDraftReplaced: (undo) => {
    set({ queuedNotice: null, draftReplacement: { id: crypto.randomUUID(), undo } });
  },
  dismissQueued: () => {
    set({ queuedNotice: null });
  },
  dismissDraftReplacement: () => {
    set({ draftReplacement: null });
  },
}));
