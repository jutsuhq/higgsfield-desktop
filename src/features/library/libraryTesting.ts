import type { CardMenu_Serialize as CardMenu, LibraryCard_Serialize, ResultDetail_Serialize, ViralityView_Serialize as ViralityView } from "../../bindings";

/** A card menu with nothing past the actions every card has. */
export const emptyMenu: CardMenu = {
  canCompare: false,
  canCopyImage: false,
  hasOriginal: false,
  followUp: null,
  canRerun: false,
  canEditVideo: false,
  canEditIn: false,
  canJoin: false,
};

/** A result without a score. */
export const noScore: ViralityView = {
  isScoring: false,
  badge: null,
  help: null,
  accessibilityLabel: null,
  headline: null,
  details: [],
  disclaimer: null,
  curve: [],
  reportUrl: null,
  failure: null,
  blocker: null,
};

/** A video card as Rust sends it, for tests. */
export function card(id: string, title: string, overrides: Partial<LibraryCard_Serialize> = {}): LibraryCard_Serialize {
  return {
    id,
    kind: "video",
    title,
    subtitle: "480p · $1.91 · 36 min ago",
    accessibilityLabel: title,
    modeTag: "Motion",
    isDraft: true,
    durationText: "6 s",
    aspectText: null,
    prompt: `${title} prompt`,
    isStarred: false,
    canStar: true,
    culling: "unreviewed",
    filePath: `/home/me/Videos/${id}.mp4`,
    mediaUrl: `http://127.0.0.1:1/${id}.mp4`,
    needsImport: false,
    isDownloading: false,
    isUnavailable: false,
    variantGroup: null,
    variantCount: null,
    isVariantPrimary: false,
    fit: "fill",
    requestId: null,
    menu: emptyMenu,
    batch: null,
    virality: null,
    ...overrides,
  };
}

/** A result's page as Rust sends it, for tests. */
export function resultDetail(id: string, overrides: Partial<ResultDetail_Serialize> = {}): ResultDetail_Serialize {
  return {
    id,
    title: "rooftop · Claymation",
    headline: "36 min ago · Draft",
    prompt: "A clay rooftop at dusk",
    facts: ["Motion", "Genjutsu", "480p"],
    rangeText: "00:04.0 – 00:10.0",
    sourceLine: "clip.mov",
    referencePreviews: [],
    billing: "$1.91 · API key",
    timing: "Queued 12 s · Rendered 1 min 4 s",
    requestId: "req-1",
    correlationId: null,
    isStarred: false,
    starTitle: "Star",
    canStar: true,
    culling: "unreviewed",
    isDraft: true,
    result: { path: `/home/me/Videos/${id}.mp4`, url: `http://127.0.0.1:1/${id}.mp4`, exists: true },
    remoteUrl: null,
    needsImport: false,
    source: { file: { path: "/home/me/clip.mov", url: "http://127.0.0.1:1/clip.mov", exists: true }, start: 4, end: 10 },
    aspectRatio: 16 / 9,
    chain: [],
    madeFrom: [],
    usedIn: [],
    inputs: [{ title: "Source video", fileName: "rooftop – Source.mov", isSource: true, reference: null, isHere: true }],
    menu: emptyMenu,
    virality: noScore,
    enhance: null,
    operationSource: null,
    ...overrides,
  };
}
