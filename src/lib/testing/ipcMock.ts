import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import { act } from "@testing-library/react";

import type {
  ConnectionChanged_Serialize,
  ConnectionDetails_Serialize,
  ConnectionSnapshot_Serialize,
  Preferences_Serialize,
  QueueSnapshot_Serialize,
  CreateSnapshot_Serialize,
  LibrarySnapshot_Serialize,
  ImageSnapshot_Serialize,
  PromptHistoryView_Serialize,
  SidebarLayout,
} from "../../bindings";
import { defaultShortcutBindings } from "../shortcuts/testing";

/** The sidebar as a fresh install shows it. */
export const sidebarShown: SidebarLayout = {
  isCollapsed: false,
  width: 228,
  minWidth: 228,
  maxWidth: 320,
  toggleHint: "Hide sidebar (Ctrl+Shift+S)",
  toggleLabel: "Hide Sidebar",
};

/** A fake command: gets the invoke's arguments, returns its result or throws its `AppError`. */
export type CommandHandler = (args: Record<string, unknown>) => unknown;

/** Rust's snake_case command names, as `invoke` receives them. */
export type CommandHandlers = Partial<Record<string, CommandHandler>>;

export const disconnected: ConnectionSnapshot_Serialize = {
  kind: null,
  activity: { kind: "idle" },
  needsAttention: false,
  alert: null,
  plate: null,
  signOut: null,
  credentialStore: "Windows Credential Manager",
  isRestoring: false,
};

export const connectedWithKey: ConnectionSnapshot_Serialize = {
  ...disconnected,
  kind: "apiKey",
  plate: {
    badge: "API key",
    tier: null,
    amount: "Per job",
    unit: "billed in USD",
    warning: null,
    isAPIKey: true,
    accessibilityLabel: "API key, billed in USD per job",
  },
  signOut: {
    title: "Remove the API key?",
    message: "The key is deleted from Windows Credential Manager. Queued jobs pause until you connect again.",
    confirmTitle: "Remove",
  },
};

/** A Queue with nothing in it. */
export const emptyQueue: QueueSnapshot_Serialize = {
  activeCount: 0,
  slotsSummary: "0 of 2 slots busy",
  canPauseAll: false,
  canResumeAll: false,
  isChangingAll: false,
  isBusy: false,
  active: [],
  problems: [],
  finished: [],
};

const keyDetails: ConnectionDetails_Serialize = {
  rows: [
    { label: "Key ID", value: "kid1…1234", isMonospaced: true },
    { label: "Secret", value: "••••••••••••", isMonospaced: false },
    { label: "Stored in", value: "Windows Credential Manager", isMonospaced: false },
  ],
  unlimitedText: null,
  usesUnlimited: true,
};

/**
 * Answers the app's commands in a test without Rust, and lets `emitEvent` reach `listen`. Unlisted commands answer
 * as a connected API key with default settings; `handlers` override any of them. Returns the calls, in order.
 */
export function mockCommands(handlers: CommandHandlers = {}) {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  let side = emptyImages.side;
  const defaults: CommandHandlers = {
    connection_status: () => connectedWithKey,
    connection_details: () => keyDetails,
    get_preferences: (): Preferences_Serialize => ({}),
    shortcut_bindings: () => defaultShortcutBindings,
    sidebar_layout: () => sidebarShown,
    queue_snapshot: () => emptyQueue,
    create_snapshot: () => emptyCreate,
    library_snapshot: () => emptyLibrary,
    library_counts: () => emptyLibrary.counts,
    // The switch is kept, as Rust keeps it in the draft.
    images_snapshot: () => ({ ...emptyImages, side }),
    images_set_side: (args) => {
      side = args.side as ImageSnapshot_Serialize["side"];
      return null;
    },
    templates_list: () => [],
    on_device_runs: () => [],
    prompts_history: (): PromptHistoryView_Serialize => ({ rows: [], emptyText: "Prompts of jobs you submit appear here." }),
  };
  mockIPC(
    (command, payload) => {
      const args = (payload ?? {}) as Record<string, unknown>;
      calls.push({ command, args });
      const handler = handlers[command] ?? defaults[command];
      return handler ? handler(args) : null;
    },
    { shouldMockEvents: true },
  );
  return calls;
}

/** Sends a Rust event to the window, as `ConnectionChanged.emit` does. */
export async function emitEvent(name: string, payload: unknown) {
  await act(async () => {
    await emit(name, payload);
  });
}

export async function emitConnection(snapshot: ConnectionSnapshot_Serialize) {
  const payload: ConnectionChanged_Serialize = { snapshot };
  await emitEvent("connection-changed", payload);
}

/** A Library with nothing in it. */
export const emptyLibrary: LibrarySnapshot_Serialize = {
  cards: [],
  counts: { videos: 0, images: 0, all: 0 },
  options: {
    modes: [],
    models: [],
    resolutions: [],
    hasDraftsAndFinals: false,
    imageModels: [],
    imageAspects: [],
  },
  fields: [],
  isFilterActive: false,
  isOriginEmpty: true,
  empty: {
    title: "No results yet",
    detail: "Finished videos are saved to your output folder and appear here.",
    action: null,
  },
  placeholder: null,
  history: null,
  canUndo: false,
};

/** Create with nothing in it: Motion on Kling, waiting for a source video. */
export const emptyCreate: CreateSnapshot_Serialize = {
  revision: 1,
  tagline: "Move your characters, products or clothes with motion from any clip.",
  tabs: [
    { value: "motionControl", title: "Motion", unavailableReason: null },
    { value: "extend", title: "Extend", unavailableReason: null },
  ],
  tab: "motionControl",
  tasks: [],
  mode: "motionControl",
  models: [{ id: "kling-3.0", title: "Kling 3.0", detail: null, unavailableReason: null }],
  model: "kling-3.0",
  source: null,
  trim: null,
  references: {
    title: "Characters",
    hint: "Who or what moves: a person, product or outfit",
    items: [],
    countText: "0 / 1",
    canAdd: true,
    disabledReason: null,
    roles: [],
  },
  frames: null,
  prompt: {
    title: "Prompt · optional",
    hint: "Describe the scene",
    counter: "0 / 2 500",
    text: "",
    isTooLong: false,
  },
  resolution: null,
  klingQuality: null,
  bitrate: null,
  orientation: null,
  videoBackground: null,
  cinema: null,
  audio: {
    title: "Audio",
    options: [
      { value: "original", title: "Original", unavailableReason: null },
      { value: "mute", title: "Mute", unavailableReason: null },
      { value: "fadeOut", title: "Fade out", unavailableReason: null },
    ],
    selected: "original",
    detail: "From the source",
    unavailableReason: null,
    hint: null,
  },
  addedLength: null,
  clipLength: null,
  aspectRatio: null,
  draftLength: null,
  upscale: null,
  variants: null,
  speech: null,
  presets: null,
  batch: null,
  characterSets: null,
  showsMotionLibrary: false,
  offersHiggsfieldReferences: false,
  linkDownload: null,
  sourceDetail: "4–30 s",
  sourceHint: "Any format · trimmed to 4–30 s",
  issues: ["Add a source video"],
  submit: {
    blocker: "Add a source video",
    costLine: null,
    generate: { title: "Generate", price: null, blocker: "Add a source video" },
    draft: { title: "Draft 480p", price: null, blocker: "Add a source video" },
    primaryIsDraft: false,
    isSubmitting: false,
  },
};

/** The Image side of Create with nothing in it, behind the video side. */
export const emptyImages: ImageSnapshot_Serialize = {
  side: "video",
  tagline: "Make still images from a prompt. Every image is saved to Library › Images.",
  prompt: "",
  unavailableReason: null,
  loadingModels: false,
  models: [{ id: "soul-2", title: "Soul 2 · $0.0032 an image" }],
  modelId: "soul-2",
  aspectRatios: ["1:1", "16:9"],
  aspectRatio: "1:1",
  variantCounts: [1, 4],
  count: 1,
  input: null,
  generate: { isGenerating: false, enabled: false, price: null, hint: "" },
  notice: { text: "Write a prompt", isQuiet: false },
  runs: [],
  requests: [],
  images: [],
  current: null,
  pendingTiles: 0,
  running: null,
  unsaved: null,
  saveFailure: null,
};
