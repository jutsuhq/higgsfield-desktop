/**
 * Every query key in one place. A key is a path: invalidating `connection.all` also invalidates
 * `connection.details`.
 */
export const queryKeys = {
  /** The About window. */
  about: ["about"] as const,
  connection: {
    all: ["connection"] as const,
    status: ["connection", "status"] as const,
    details: ["connection", "details"] as const,
  },
  preferences: ["preferences"] as const,
  queue: {
    all: ["queue"] as const,
    snapshot: ["queue", "snapshot"] as const,
  },
  settings: {
    /** What Rust computes from the preferences; `PreferencesChanged` fetches it again. */
    fromPreferences: ["settings", "preferences"] as const,
    general: ["settings", "preferences", "general"] as const,
    output: ["settings", "preferences", "output"] as const,
    notifications: ["settings", "preferences", "notifications"] as const,
    agents: ["settings", "preferences", "agents"] as const,
    shortcuts: ["settings", "preferences", "shortcuts"] as const,
    shortcutBindings: ["settings", "preferences", "shortcutBindings"] as const,
    /** The main window's sidebar: shown or hidden, and its width. */
    sidebar: ["settings", "preferences", "sidebar"] as const,
    outputName: (pattern: string) => ["settings", "outputName", pattern] as const,
    /** Disk use; the preferences don't change it. */
    storage: ["settings", "storage"] as const,
    storageSummary: ["settings", "storage", "summary"] as const,
    oldDrafts: (days: number | null) => ["settings", "storage", "oldDrafts", days] as const,
    subtitles: ["settings", "subtitles"] as const,
    /** Settings › Spending of a period; the cap and low-credit fields follow the preferences. */
    spending: (period: string) => ["settings", "preferences", "spending", period] as const,
    creditHistory: ["settings", "creditHistory"] as const,
  },
  create: {
    all: ["create"] as const,
    snapshot: ["create", "snapshot"] as const,
    motion: ["create", "motion"] as const,
    higgsfield: (source: string) => ["create", "higgsfield", source] as const,
  },
  library: {
    /** Everything the Library shows; `LibraryChanged` fetches it again. */
    all: ["library"] as const,
    snapshot: (request: unknown) => ["library", "snapshot", request] as const,
    counts: ["library", "counts"] as const,
    thumbnail: (id: string) => ["library", "thumbnail", id] as const,
    detail: (id: string) => ["library", "detail", id] as const,
    image: (id: string, versionId: string | null) => ["library", "image", id, versionId] as const,
  },
  subtitles: {
    /** A video's Subtitles sheet and editor; `SubtitlesChanged` fetches them again. */
    all: ["subtitles"] as const,
    sheet: (id: string, language: string | null, engine: string | null) =>
      ["subtitles", "sheet", id, language, engine] as const,
    editor: (id: string) => ["subtitles", "editor", id] as const,
    /** The runs the Library's cards show. */
    runs: ["subtitles", "runs"] as const,
  },
  images: {
    /** The Image side of Create; `ImagesChanged` fetches it again. */
    all: ["images"] as const,
    snapshot: ["images", "snapshot"] as const,
    /** Create Character; `ImagesChanged` fetches it again. */
    characters: ["images", "characters"] as const,
    /** References › Generate…'s picks; `ImagesChanged` and `CreateChanged` fetch them again. */
    referencePicks: ["images", "referencePicks"] as const,
  },
  templates: (side: string) => ["templates", side] as const,
  prompts: (query: string) => ["prompts", query] as const,
  /** The tray panel; `QueueChanged`, `ConnectionChanged`, `PreferencesChanged` and `LibraryChanged` fetch it again. */
  tray: ["tray"] as const,
  /** Enhance on this computer; `OnDeviceRunsChanged` fetches the runs again. */
  onDevice: {
    all: ["onDevice"] as const,
    runs: ["onDevice", "runs"] as const,
    sheet: (id: string, kind: string, motion: string | null) => ["onDevice", "sheet", id, kind, motion] as const,
  },
};
