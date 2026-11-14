import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import {
  CircleMinus,
  Columns2,
  Copy,
  Eye,
  EyeOff,
  Film,
  Folder,
  ImagePlus,
  ListFilter,
  Package,
  RefreshCw,
  Share,
  Star,
  StarOff,
  Trash2,
  TriangleAlert,
  UserRound,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import {
  commands,
  type BatchSummary,
  type BulkActions,
  type ComparePick,
  type CullingAction,
  type HistoryPlaceholder,
  type LibraryCard_Serialize,
  type LibrarySnapshot_Serialize,
  type MakeVideoChoice,
  type MakeVideoOutcome_Serialize,
} from "../../bindings";
import { AlertDialog } from "../../app/Dialog";
import { useToastStore } from "../../app/toastStore";
import { useLibraryCounts, type LibraryCounts } from "../../app/liveData";
import { EmptyState, PageHeader } from "../../design/Components";
import { cx } from "../../design/cx";
import { PagedFeedFooter } from "../../design/PagedFeedFooter";
import { FilterMenu, FilterToggle, SearchField, SegmentedControl, type Option } from "../../design/Pickers";
import { Spinner } from "../../design/Spinner";
import { Button, focusRing } from "../../design/Styles";
import { isEditingText } from "../../design/textFocus";
import { useCommand } from "../../lib/ipc";
import { queryKeys } from "../../lib/queryKeys";
import { usePageCommand } from "../../lib/shortcuts/pageCommands";
import { useShortcut } from "../../lib/shortcuts/useShortcut";
import { loadJob, openFollowUp, useCreateNotice } from "../create/createData";
import { CullingButtons } from "./CullingButtons";
import { RenderFinalsSheet, copyVideos } from "./followUps";
import { LibraryCard, type CardHandlers, type CardRun } from "./LibraryCard";
import { DELIVERY_TITLE, DeliverySetsSheet, NEW_SET_TITLE } from "./Delivery";
import { ContextMenu, DeleteDialog, ImageDetailsDialog, Preview, type MenuItem } from "./LibraryDialogs";
import { cull, useLibrarySnapshot, useLibraryView, type Filter } from "./libraryData";
import { OnDeviceSheet } from "./OnDeviceSheet";
import { SubtitlesSheet } from "./Subtitles";
import {
  chosen,
  cullingTargets,
  emptySelection,
  extended,
  movedCursor,
  selectedAll,
  selectionTitle,
  toggled,
  type CursorMove,
  type Selection,
} from "./selection";
import { VirtualGrid } from "./VirtualGrid";

type Kind = keyof LibraryCounts;
type MenuEntry = MenuItem | "divider";

/** Share has no system sheet here: it shows the files in the file manager, to drag into the app to share with. */
const shareHint = "Show in the file manager to share from there";
/** Edit in… opens a copy in the default app for its type; the platform has no app picker for it. */
const editInHint =
  "Opens a copy in the default app. The original stays as it is; bring the edit back with Import as New Version.";

const origins = [
  { value: "local", title: "On this computer" },
  { value: "higgsfield", title: "From Higgsfield" },
] as const;

const kinds: readonly { value: Kind; title: string }[] = [
  { value: "all", title: "All" },
  { value: "video", title: "Videos" },
  { value: "image", title: "Images" },
];

const sorts: Option<Filter["sort"]>[] = [
  { value: "newest", title: "Newest first" },
  { value: "oldest", title: "Oldest first" },
  { value: "highestPrice", title: "Highest price" },
];

const finishes: Option<Filter["finish"]>[] = [
  { value: "all", title: "Drafts and finals" },
  { value: "drafts", title: "Drafts only" },
  { value: "finals", title: "Finals only" },
];

/** `CullingAction.title`. */
const cullingTitles = { keep: "Keep", reject: "Reject", clear: "Clear Keep or Reject" } as const;

const arrows: Partial<Record<string, CursorMove>> = {
  ArrowLeft: "left",
  ArrowRight: "right",
  ArrowUp: "up",
  ArrowDown: "down",
};

/** The Library (`LibraryView.swift`): its header, the origin switch, the kind tabs, the filters and the grid. */
export function LibraryPage() {
  const navigate = useNavigate();
  const view = useLibraryView();
  const { origin, kind, filter, expandedGroups } = view;
  const snapshot = useLibrarySnapshot({ origin, kind, filter, expandedGroups }).data;
  const localCounts = useLibraryCounts();
  const counts: LibraryCounts = snapshot
    ? { all: snapshot.counts.all, video: snapshot.counts.videos, image: snapshot.counts.images }
    : localCounts;
  const cards = useMemo(() => snapshot?.cards ?? [], [snapshot]);
  const order = useMemo(() => cards.map((card) => card.id), [cards]);
  const byId = useMemo(() => new Map(cards.map((card) => [card.id, card])), [cards]);
  const [selection, setSelection] = useState<Selection>(emptySelection);
  const [cursor, setCursor] = useState<string | null>(null);
  const [columns, setColumns] = useState(1);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  const [deleting, setDeleting] = useState<{ ids: string[]; images: boolean } | null>(null);
  // Remove from Library…'s question for images whose files are gone, as Rust words it.
  const [removing, setRemoving] = useState<{ ids: string[]; title: string } | null>(null);
  // Subtitles… from a card's menu, and Upscale on This Computer… from an image's.
  const [subtitlesFor, setSubtitlesFor] = useState<LibraryCard_Serialize | null>(null);
  const [upscaling, setUpscaling] = useState<string | null>(null);
  // Compare in the header: two results picked in the grid (`ComparePicker`), as Rust keeps the picks. Images are
  // picked among images, results among results.
  const [pick, setPick] = useState<ComparePick | null>(null);
  const [pickKind, setPickKind] = useState<"video" | "image">("video");
  const [finals, setFinals] = useState<string[] | null>(null);
  const [preview, setPreview] = useState<LibraryCard_Serialize | null>(null);
  const [details, setDetails] = useState<string | null>(null);
  // The delivery sets sheet and the set it opens on.
  const [deliverySets, setDeliverySets] = useState<{ selection: string | null } | null>(null);
  // Make Video’s question when more than one role fits the selected model, and the images it is about.
  const [roleQuestion, setRoleQuestion] = useState<{
    ids: string[];
    ask: Extract<MakeVideoOutcome_Serialize, { kind: "ask" }>;
  } | null>(null);
  const isModal =
    menu !== null ||
    deleting !== null ||
    removing !== null ||
    preview !== null ||
    details !== null ||
    subtitlesFor !== null ||
    upscaling !== null ||
    finals !== null ||
    deliverySets !== null ||
    roleQuestion !== null;
  // The runs on this computer each card shows over its frame (`OnDeviceRunBadge`).
  const onDeviceRuns = useCommand(queryKeys.onDevice.runs, commands.onDeviceRuns).data ?? [];
  const subtitleRuns = useCommand(queryKeys.subtitles.runs, commands.subtitlesRuns).data ?? [];
  const history = snapshot?.history ?? null;
  const canRefresh = origin === "higgsfield" && history?.canRefresh === true;
  const loadedHistory = useRef(false);
  const setView = view.set;

  useEffect(() => {
    setView({ order: cards.filter((card) => card.kind === "video").map((card) => card.id) });
  }, [cards, setView]);

  // The account's first page loads when the Higgsfield origin is first shown.
  useEffect(() => {
    if (!canRefresh || loadedHistory.current) return;
    loadedHistory.current = true;
    void commands.libraryLoadHistory(true);
  }, [canRefresh]);

  /** Starts picking results or images to compare, with `picked` already picked. */
  const startPicking = (kind: "video" | "image", picked: string[] = []) => {
    setSelection(emptySelection);
    void commands.libraryComparePick(picked, null).then((started) => {
      setPickKind(kind);
      setPick(started);
    });
  };
  // A result only on Higgsfield is brought here first, as the grid does before it compares one; an image only on
  // Higgsfield is shown from its link. An image whose file is missing can't be picked.
  const togglePick = (card: LibraryCard_Serialize) => {
    if (!pick || card.kind !== pickKind || (card.kind === "image" && !card.menu.canCompare)) return;
    const picked = pick.picked;
    void (card.kind === "video" && card.needsImport ? commands.libraryDownload(card.id) : Promise.resolve(null))
      .then(() => commands.libraryComparePick(picked, card.id))
      .then(setPick);
  };
  /** Compare… on an image (`compareImage`): picking starts with it, or it joins the picks. */
  const compareImage = (card: LibraryCard_Serialize) => {
    if (pick && pickKind === "image") {
      if (pick.picked[0] !== card.id) togglePick(card);
      return;
    }
    startPicking("image", [card.id]);
  };
  const comparePicked = () => {
    if (!pick?.isReady) return;
    setPick(null);
    const ids = pick.picked.join(",");
    if (pickKind === "image") void navigate({ to: "/library/compare-images/$ids", params: { ids } });
    else void navigate({ to: "/library/compare/$ids", params: { ids } });
  };

  const setFilter = (change: Partial<Filter>) => {
    setView({ filter: { ...filter, ...change } });
  };
  const targets = () => cullingTargets(order, selection, cursor);

  // A result only on Higgsfield is downloaded into the library first, then opens like any other (`withImported`); a
  // failed download says so and leaves the grid as it is.
  const open = (card: LibraryCard_Serialize) => {
    if (card.batch !== null) void navigate({ to: "/library/batch/$id", params: { id: card.id } });
    else if (card.kind === "video" && card.needsImport) {
      if (card.isDownloading) return;
      void commands
        .libraryDownload(card.id)
        .then(() => commands.libraryDetail(card.id))
        .then((detail) => {
          if (!detail.needsImport) void navigate({ to: "/library/result/$id", params: { id: card.id } });
        })
        .catch(() => undefined);
    } else if (card.kind === "video") void navigate({ to: "/library/result/$id", params: { id: card.id } });
    else setDetails(card.id);
  };
  /** Star when any of them isn't starred yet, else unstar them all; Rust decides (`LibraryBulkActions.starsAll`). */
  const star = (ids: string[]) => {
    if (ids.length > 0) void commands.libraryToggleStar(ids);
  };
  /** New Delivery Set of the chosen items; the sheet opens on it. */
  const newDeliverySet = (ids: string[]) => {
    commands
      .deliveryNew(ids)
      .then((id) => {
        setSelection(emptySelection);
        setDeliverySets({ selection: id });
      })
      .catch(() => undefined);
  };
  // The current card alone moves on after a verdict (`LibraryCursor.advanced`); a selection keeps it.
  const applyCulling = (action: CullingAction) => {
    const current = chosen(selection, order).length === 0 ? cursor : null;
    void cull(action, targets(), current === null ? null : { id: current, order, showsRejected: filter.showsRejected }).then(
      (outcome) => {
        if (current !== null) setCursor(outcome.cursor ?? null);
      },
    );
  };
  const undoCulling = () => {
    void commands.libraryUndoCulling().then((restored) => {
      if (restored !== null) setCursor(restored);
    });
  };

  const batchMenu = (card: LibraryCard_Serialize, batch: BatchSummary): MenuEntry[] => [
    {
      title: selection.ids.has(card.id) ? "Deselect" : "Select",
      run: () => { setSelection((current) => toggled(current, card.id)); },
    },
    { title: "Share…", hint: shareHint, run: () => void commands.libraryShowInFolder(batch.memberIds) },
    ...(batch.canRenderFinals
      ? [{ title: "Render Finals for Starred…", run: () => { setFinals(batch.memberIds); } }]
      : []),
    "divider",
    {
      title: "Delete Unstarred…",
      hint: "Keep the starred results and delete the rest of this batch",
      isDestructive: true,
      isDisabled: batch.unstarredIds.length === 0,
      run: () => { setDeleting({ ids: batch.unstarredIds, images: false }); },
    },
  ];

  /** Keep, Reject and Clear for the card or the selection it belongs to (`LibraryView.cullingMenu`). */
  const cullingMenu = (ids: string[]): MenuEntry[] => [
    "divider",
    ...(["keep", "reject", "clear"] as const).map((action) => {
      const title = cullingTitles[action];
      return {
        title: ids.length > 1 ? `${title} ${String(ids.length)} Results` : title,
        run: () => void cull(action, ids),
      };
    }),
  ];

  /** Show or hide a group's variants, make one its face, and compare them (`LibraryView.variantMenu`). */
  const variantMenu = (card: LibraryCard_Serialize): MenuEntry[] => {
    const group = card.variantGroup;
    if (group === null) return [];
    const items: MenuEntry[] = ["divider"];
    if (expandedGroups.includes(group)) {
      items.push({
        title: "Hide Variants",
        run: () => { setView({ expandedGroups: expandedGroups.filter((id) => id !== group) }); },
      });
      if (!card.isVariantPrimary) {
        items.push({ title: "Make Primary", run: () => void commands.libraryMakePrimary(card.id) });
      }
    } else if (card.variantCount !== null) {
      items.push({
        title: `Show ${String(card.variantCount)} Variants`,
        run: () => { setView({ expandedGroups: [...expandedGroups, group] }); },
      });
    }
    // Videos open as the comparison grid of the group; images step through it two at a time.
    items.push({
      title: "Compare Variants",
      run: () =>
        void (card.kind === "video"
          ? navigate({ to: "/library/batch/$id", params: { id: group } })
          : navigate({ to: "/library/compare-variants/$group", params: { group } })),
    });
    return items;
  };

  // The menu of a card among several selected ones: the selection bar's actions, then Deselect All (`LibraryBulkMenu`).
  const bulkMenu = (ids: string[], actions: BulkActions | undefined): MenuEntry[] => {
    const items: MenuEntry[] = [];
    if (actions && actions.starrableIds.length > 0) {
      items.push({ title: actions.starsAll ? "Star" : "Unstar", run: () => { star(ids); } });
    }
    const files = actions?.fileIds ?? [];
    if (files.length > 0) {
      items.push(
        { title: "Show in File Manager", run: () => void commands.libraryShowInFolder(files) },
        { title: "Copy Videos", hint: "Copy videos", run: () => void copyVideos(files) },
        { title: "Share…", hint: shareHint, run: () => void commands.libraryShowInFolder(files) },
      );
    }
    if (actions && actions.deletableIds.length > 0) {
      const deletable = actions.deletableIds;
      items.push({
        title: "Delete from Library…",
        isDestructive: true,
        run: () => { setDeleting({ ids: deletable, images: false }); },
      });
    }
    items.push("divider", { title: "Deselect All", run: () => { setSelection(emptySelection); } });
    return items;
  };

  /** A result card's menu (`LibraryCardMenu`): the result page's actions that make sense without opening it. */
  const videoMenu = (card: LibraryCard_Serialize): MenuEntry[] => {
    const hasFile = card.filePath !== null && !card.isUnavailable;
    const { menu: actions } = card;
    const local = () => (card.needsImport ? commands.libraryDownload(card.id) : Promise.resolve(null));
    const items: MenuEntry[] = [];
    if (card.needsImport && !card.isDownloading) {
      items.push({ title: "Download to Library", run: () => void commands.libraryDownload(card.id) });
    }
    if (card.canStar) items.push({ title: card.isStarred ? "Unstar" : "Star", run: () => { star([card.id]); } });
    if (actions.canCompare) items.push({ title: "Compare…", run: () => { startPicking("video", [card.id]); } });
    if (actions.hasOriginal) {
      items.push({
        title: "Compare with Original",
        hint: "Play this version beside the one it was made from",
        run: () => void navigate({ to: "/library/compare-original/$id", params: { id: card.id } }),
      });
    }
    // A result only on Higgsfield is downloaded first, then shown (`withImported`).
    items.push({
      title: "Show in File Manager",
      run: () => void local().then(() => commands.libraryShowInFolder([card.id])),
    });
    if (hasFile) items.push({ title: "Share…", hint: shareHint, run: () => void commands.libraryShowInFolder([card.id]) });
    items.push({
      title: "Export Short Preview…",
      hint: "The first 5 seconds at 480p without sound, small enough to send in a messenger",
      run: () => void local().then(() => commands.libraryExportShortPreview(card.id)),
    });
    if (card.prompt !== "") {
      items.push({ title: "Copy Prompt", run: () => void navigator.clipboard.writeText(card.prompt) });
    }
    items.push(...followUps(card));
    const run = onDeviceRuns.find((row) => row.itemId === card.id);
    if (run) items.push({ title: `Cancel ${run.title}`, run: () => void commands.onDeviceCancel(run.id) });
    const subtitleRun = subtitleRuns.find((row) => row.itemId === card.id);
    if (subtitleRun) {
      items.push({ title: "Cancel Subtitles", run: () => void commands.subtitlesCancel(subtitleRun.id) });
    } else if (hasFile && !card.needsImport) {
      items.push({
        title: "Subtitles…",
        hint: "Make subtitles from the speech in this video, free on this computer",
        run: () => { setSubtitlesFor(card); },
      });
    }
    if (actions.canEditIn) items.push(...outsideEdit(card));
    items.push("divider", {
      title: selection.ids.has(card.id) ? "Deselect" : "Select",
      run: () => { setSelection((current) => toggled(current, card.id)); },
    });
    if (!card.needsImport) {
      items.push("divider", {
        title: "Delete from Library…",
        isDestructive: true,
        run: () => { setDeleting({ ids: [card.id], images: false }); },
      });
    }
    return items;
  };

  /** Edit in… and Import as New Version… (`OutsideEditButtons`). */
  const outsideEdit = (card: LibraryCard_Serialize): MenuEntry[] => [
    { title: "Edit in…", hint: editInHint, run: () => void commands.libraryEditIn(card.id) },
    {
      title: "Import as New Version…",
      hint: "Bring an edited file back as this item's next version, linked to the one it was made from",
      run: () => void commands.libraryImportVersion(card.id),
    },
  ];

  /**
   * An image card's menu (`LibraryImageMenu`), for the card or the selected images it is one of: a saved image's,
   * one whose file can't be found, or an account image's. `upscales` says whether Upscale on This Computer… fits it.
   */
  const imageMenu = (card: LibraryCard_Serialize, chosenImages: LibraryCard_Serialize[], upscales: boolean): MenuEntry[] => {
    const isSelected = selection.ids.has(card.id);
    const select: MenuEntry[] =
      pick === null
        ? ["divider", { title: isSelected ? "Deselect" : "Select", run: () => { setSelection((current) => toggled(current, card.id)); } }]
        : [];
    const copyPrompt = (prompt: string): MenuEntry[] =>
      prompt === "" ? [] : [{ title: "Copy Prompt", run: () => void navigator.clipboard.writeText(prompt) }];
    if (card.needsImport) {
      return [
        { title: card.isStarred ? "Unlike" : "Like", run: () => void commands.librarySetStarred([card.id], !card.isStarred) },
        { title: "Download to Library", run: () => void commands.libraryDownload(card.id) },
        { title: "Compare…", run: () => { compareImage(card); } },
        { title: "Make Video…", run: () => void makeVideo([card.id], false) },
        { title: "Use as Reference", run: () => void makeVideo([card.id], true) },
        { title: "Details…", run: () => { setDetails(card.id); } },
        { title: "Quick Look", run: () => { setPreview(card); } },
        ...copyPrompt(card.prompt),
        ...select,
      ];
    }
    const saved = chosenImages.filter((image) => !image.needsImport && image.isUnavailable === card.isUnavailable);
    const ids = saved.map((image) => image.id);
    const starsAll = saved.every((image) => image.isStarred);
    const starItem: MenuEntry = {
      title: starsAll ? "Unstar" : "Star",
      run: () => void commands.librarySetStarred(ids, !starsAll),
    };
    if (card.isUnavailable) {
      return [
        starItem,
        ...(ids.length === 1 ? [{ title: "Details…", run: () => { setDetails(card.id); } }] : []),
        ...(ids.length === 1 ? copyPrompt(card.prompt) : []),
        "divider",
        {
          title: "Remove from Library…",
          isDestructive: true,
          run: () => void commands.libraryRemovalTitle(ids.length).then((title) => { setRemoving({ ids, title }); }),
        },
      ];
    }
    const items: MenuEntry[] = [
      starItem,
      { title: ids.length > 1 ? `Make Video from ${String(ids.length)}…` : "Make Video…", run: () => void makeVideo(ids, false) },
      { title: ids.length > 1 ? `Use ${String(ids.length)} as References` : "Use as Reference", run: () => void makeVideo(ids, true) },
    ];
    if (ids.length === 1) items.push({ title: "Details…", run: () => { setDetails(card.id); } });
    items.push({ title: "Compare…", run: () => { compareImage(card); } });
    if (ids.length === 1 && card.menu.hasOriginal) {
      items.push({
        title: "Compare with Original",
        run: () => void navigate({ to: "/library/compare-image-original/$id", params: { id: card.id } }),
      });
    }
    if (ids.length === 1) {
      if (card.menu.canEditIn) items.push(...outsideEdit(card));
      const run = onDeviceRuns.find((row) => row.itemId === card.id);
      if (run) items.push({ title: `Cancel ${run.title}`, run: () => void commands.onDeviceCancel(run.id) });
      else if (upscales) items.push({ title: "Upscale on This Computer…", run: () => { setUpscaling(card.id); } });
    }
    items.push(
      { title: "Quick Look", run: () => { setPreview(card); } },
      { title: "Show in File Manager", run: () => void commands.libraryShowInFolder(ids) },
      "divider",
      { title: "Copy Image", run: () => void commands.libraryCopyImage(card.id) },
      ...(ids.length === 1 ? copyPrompt(card.prompt) : []),
      "divider",
      { title: "Move to Trash…", isDestructive: true, run: () => { setDeleting({ ids, images: true }); } },
      ...select,
    );
    return items;
  };

  /** Edit Inputs, Finalize, Render Final, Edit & Re-run, Edit… and Extend… (`LibraryCardMenu.followUps`). */
  const followUps = (card: LibraryCard_Serialize): MenuEntry[] => {
    const items: MenuEntry[] = [];
    const openResult = () => void navigate({ to: "/library/result/$id", params: { id: card.id } });
    const inCreate = (load: () => Promise<unknown>) => () => {
      void load().then(() => navigate({ to: "/create" }));
    };
    switch (card.menu.followUp) {
      case "editInputs":
        items.push({
          title: "Edit Inputs",
          hint: "Run these inputs again; the files Higgsfield still has aren't uploaded again",
          run: inCreate(() => commands.libraryDownload(card.id).then(() => loadJob(card.id))),
        });
        break;
      case "finalize":
        items.push({ title: "Finalize in 1080p…", hint: "Open the result to finalize this draft in 1080p", run: openResult });
        break;
      case "renderFinal":
        items.push({ title: "Render Final…", hint: "Open the result to render its final", run: openResult });
        break;
      case null:
        break;
    }
    if (card.menu.canRerun) items.push({ title: "Edit & Re-run", run: inCreate(() => loadJob(card.id)) });
    const path = card.filePath;
    if (card.menu.canEditVideo && path !== null) {
      const followUp = (mode: "edit" | "extend") => inCreate(() => openFollowUp(card.id, mode));
      items.push(
        { title: "Edit…", hint: "Change this result as a prompt says, with Seedance 2.5", run: followUp("edit") },
        { title: "Extend…", hint: "Continue this result past its last frame, with Seedance 2.5", run: followUp("extend") },
      );
    }
    return items;
  };

  /** Make Video… and Use as Reference: the images go to Create in the role asked for, or as references. */
  const makeVideo = (ids: string[], asReferences: boolean, choice: MakeVideoChoice | null = null) =>
    commands.libraryMakeVideo(ids, asReferences, choice).then((outcome) => {
      switch (outcome.kind) {
        case "ask":
          setRoleQuestion({ ids, ask: outcome });
          return;
        case "unavailable":
          useCreateNotice.getState().show({ title: outcome.message.title, detail: outcome.message.message });
          return navigate({ to: "/create" });
        case "placed":
          if (outcome.canUndo) {
            useToastStore.getState().showDraftReplaced(() => void commands.createUndoReplacement());
          }
          // What Create couldn't take, as dropping the files there says it (`AddOutcome`).
          if (outcome.notice !== null) {
            useCreateNotice.getState().show({ title: outcome.notice.title, detail: outcome.notice.message });
          }
          return navigate({ to: "/create" });
      }
    });

  /** The chosen cards of one kind: the videos' and the images' selections are separate (`LibraryView`). */
  const chosenOf = (kind: "video" | "image") => chosen(selection, order).filter((id) => byId.get(id)?.kind === kind);
  /** The results the chosen cards stand for: a batch card stands for its results (`LibrarySelection.bulkItems`). */
  const resultsOf = (ids: string[]) => ids.flatMap((id) => byId.get(id)?.batch?.memberIds ?? [id]);
  /** The selection with only cards of `kind` left, as a click on the other kind clears it. */
  const keepKind = (current: Selection, kind: "video" | "image"): Selection => ({
    ...current,
    ids: new Set([...current.ids].filter((id) => byId.get(id)?.kind === kind)),
  });

  const handlers: CardHandlers = {
    onActivate: (card, event) => {
      setCursor(card.id);
      if (pick) {
        togglePick(card);
        return;
      }
      const ctrl = event.ctrlKey || event.metaKey;
      if (card.kind === "image") {
        // A click selects only this image, again deselects it; Ctrl-click adds to the images picked.
        setSelection((current) => {
          const images = keepKind(current, "image");
          if (ctrl) return toggled(images, card.id);
          return images.ids.size === 1 && images.ids.has(card.id) ? emptySelection : toggled(emptySelection, card.id);
        });
        return;
      }
      if (ctrl) setSelection((current) => toggled(keepKind(current, "video"), card.id));
      else if (event.shiftKey) setSelection((current) => extended(keepKind(current, "video"), card.id, order));
      else {
        setSelection(emptySelection);
        open(card);
      }
    },
    onDoubleClick: (card) => {
      if (card.kind === "image" && !pick) setPreview(card);
    },
    onMenu: (card, event) => {
      setCursor(card.id);
      const position = { x: event.clientX, y: event.clientY };
      const kind = card.kind;
      const ids = pick === null && selection.ids.has(card.id) ? chosenOf(kind) : [card.id];
      const results = ids.length > 1 ? resultsOf(ids) : ids;
      if (card.batch !== null) {
        if (ids.length > 1) {
          void commands.libraryBulk(results).then((actions) => {
            setMenu({ ...position, items: bulkMenu(results, actions) });
          });
        } else if (pick === null) {
          setMenu({ ...position, items: batchMenu(card, card.batch) });
        }
        return;
      }
      if (kind === "video") {
        if (ids.length > 1) {
          void commands.libraryBulk(results).then((actions) => {
            setMenu({ ...position, items: [...bulkMenu(results, actions), ...cullingMenu(results), ...variantMenu(card)] });
          });
        } else {
          setMenu({ ...position, items: [...videoMenu(card), ...cullingMenu(ids), ...variantMenu(card)] });
        }
        return;
      }
      const images = ids.flatMap((id) => byId.get(id) ?? []);
      // Upscale on This Computer… is offered by the image's size, which its file says.
      const offer =
        images.length === 1 && !card.needsImport && !card.isUnavailable
          ? commands.onDeviceOffersImageUpscale(card.id).catch(() => false)
          : Promise.resolve(false);
      void offer.then((upscales) => {
        setMenu({ ...position, items: [...imageMenu(card, images, upscales), ...cullingMenu(ids), ...variantMenu(card)] });
      });
    },
  };

  // The grid's keys: arrows move the current card, Space previews it, Enter opens it, Ctrl+A picks every card, Esc
  // clears the selection, Ctrl+Z takes back the last keep or reject.
  useEffect(() => {
    if (isModal) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || isEditingText()) return;
      if (pick && (event.key === "Escape" || event.key === "Enter")) {
        event.preventDefault();
        if (event.key === "Escape") setPick(null);
        else comparePicked();
        return;
      }
      const move = arrows[event.key];
      const key = event.key.toLowerCase();
      if (move && !event.ctrlKey && !event.altKey) {
        event.preventDefault();
        setCursor((current) => movedCursor(current, move, order, columns));
      } else if (event.key === " " && cursor !== null) {
        event.preventDefault();
        const card = byId.get(cursor);
        if (card) setPreview(card);
      } else if (event.key === "Enter" && cursor !== null && !event.ctrlKey) {
        const card = byId.get(cursor);
        if (card) open(card);
      } else if (event.key === "Escape" && selection.ids.size > 0) {
        event.preventDefault();
        setSelection(emptySelection);
      } else if (event.ctrlKey && !event.shiftKey && !event.altKey && key === "a") {
        event.preventDefault();
        setSelection(selectedAll(order));
      } else if (event.ctrlKey && !event.shiftKey && !event.altKey && key === "z") {
        event.preventDefault();
        undoCulling();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  });

  useShortcut("keep", () => { applyCulling("keep"); }, !isModal);
  useShortcut("reject", () => { applyCulling("reject"); }, !isModal);
  useShortcut("clearCulling", () => { applyCulling("clear"); }, !isModal);
  // The Library menu acts on the selected results, as the selection bar does (`gridCommands`); Rust names Star or
  // Unstar and the one draft Render Final… opens.
  const menuTargets = pick ? [] : resultsOf(chosenOf("video"));
  const menuBulk = useQuery({
    queryKey: [...queryKeys.library.all, "bulk", menuTargets],
    queryFn: () => commands.libraryBulk(menuTargets),
    enabled: menuTargets.length > 0,
  }).data;
  const hasTargets = menuTargets.length > 0 && menuBulk !== undefined;
  useShortcut(
    "star",
    () => { star(menuTargets); },
    !isModal && hasTargets && menuBulk.starrableIds.length > 0,
    hasTargets ? menuBulk.starTitle : null,
  );
  useShortcut(
    "deleteResult",
    () => {
      if (menuBulk) setDeleting({ ids: menuBulk.deletableIds, images: false });
    },
    !isModal && hasTargets && menuBulk.deletableIds.length > 0,
  );
  const renderFinal = hasTargets ? menuBulk.renderFinalId : null;
  usePageCommand(
    "renderFinal",
    () => {
      const card = renderFinal === null ? undefined : byId.get(renderFinal);
      if (card) open(card);
    },
    !isModal && renderFinal !== null,
    "Render Final…",
  );
  useShortcut("refreshHistory", () => void commands.libraryLoadHistory(true), canRefresh);
  // Ctrl+F (Settings › Shortcuts) focuses the search field.
  const search = useRef<HTMLInputElement>(null);
  useShortcut("find", () => search.current?.focus(), !isModal);

  // The bars over the grid while results or images are selected (`LibraryView.selectionBars`): one for each kind.
  const pickedVideos = resultsOf(chosenOf("video"));
  const pickedImages = chosenOf("image").flatMap((id) => byId.get(id) ?? []);
  const bulk = useQuery({
    queryKey: [...queryKeys.library.all, "bulk", pickedVideos],
    queryFn: () => commands.libraryBulk(pickedVideos),
    enabled: pickedVideos.length > 0,
  }).data;
  const isEmpty = snapshot?.isOriginEmpty ?? true;
  /** An enhance run on the card, else one making its subtitles. */
  const runOf = (id: string): CardRun | null =>
    onDeviceRuns.find((row) => row.itemId === id) ?? subtitleRuns.find((row) => row.itemId === id) ?? null;

  return (
    <div className="flex h-full min-h-0 flex-col px-page pt-page-top">
      <PageHeader
        title="Library"
        subtitle="Every finished video and image, also saved to your output folder."
        trailing={
          <div className="flex items-center gap-small">
            <SegmentedControl
              isCompact
              value={origin}
              options={origins}
              onChange={(value) => {
                setView({ origin: value });
                setSelection(emptySelection);
                setPick(null);
              }}
            />
            {snapshot && !isEmpty && (
              <>
                <SearchField
                  className="w-260"
                  inputRef={search}
                  value={filter.query}
                  prompt="Search prompts and names"
                  onChange={(query) => {
                    setFilter({ query });
                  }}
                />
                {pick === null && (
                  <Button
                    variant="icon"
                    icon={Columns2}
                    title="Compare"
                    hint={kind === "image" ? "Compare two images" : "Compare two results"}
                    onClick={() => {
                      startPicking(kind === "image" ? "image" : "video");
                    }}
                  />
                )}
              </>
            )}
            <Button
              variant="icon"
              icon={Package}
              title={DELIVERY_TITLE}
              hint="Open delivery sets"
              onClick={() => {
                setDeliverySets({ selection: null });
              }}
            />
            {canRefresh ? (
              <Button
                variant="icon"
                icon={RefreshCw}
                title="Refresh"
                hint="Refresh (Ctrl+R)"
                disabled={history.isLoading}
                onClick={() => void commands.libraryLoadHistory(true)}
              />
            ) : (
              // Keeps the row as wide on both origins, so the header never moves under the title on one of them.
              <span aria-hidden className="size-34 shrink-0" />
            )}
          </div>
        }
      />
      <div className="pb-small">
        <LibraryKindTabs
          kind={kind}
          counts={counts}
          onChange={(value) => {
            setView({ kind: value });
            setPick(null);
          }}
        />
      </div>
      {snapshot && !isEmpty && snapshot.fields.length > 0 && (
        <FilterBar snapshot={snapshot} filter={filter} setFilter={setFilter} />
      )}
      {pick && (
        <ComparePickBar
          pick={pick}
          onCancel={() => {
            setPick(null);
          }}
          onCompare={comparePicked}
        />
      )}
      {pick === null && pickedVideos.length > 0 && kind !== "image" && (
        <SelectionBar
          count={pickedVideos.length}
          bulk={bulk}
          onDone={() => {
            setSelection(emptySelection);
          }}
          onCull={(action) => void cull(action, pickedVideos)}
          onNewDeliverySet={() => {
            newDeliverySet(pickedVideos);
          }}
          onStar={() => {
            star(pickedVideos);
          }}
          onShow={() => void commands.libraryShowInFolder(bulk?.fileIds ?? [])}
          onCopy={() => void copyVideos(bulk?.fileIds ?? [])}
          onDelete={() => {
            if (bulk) setDeleting({ ids: bulk.deletableIds, images: false });
          }}
        />
      )}
      {pick === null && pickedImages.length > 0 && kind !== "video" && (
        <ImageSelectionBar
          chosen={pickedImages}
          onDone={() => {
            setSelection(emptySelection);
          }}
          onCull={(action) => void cull(action, pickedImages.map((image) => image.id))}
          onNewDeliverySet={() => {
            newDeliverySet(pickedImages.map((image) => image.id));
          }}
          onMakeVideo={(ids, asReferences) => void makeVideo(ids, asReferences)}
          onCompare={(ids) => {
            startPicking("image", ids);
          }}
          onTrash={(ids) => {
            setDeleting({ ids, images: true });
          }}
          onRemove={(ids) =>
            void commands.libraryRemovalTitle(ids.length).then((title) => {
              setRemoving({ ids, title });
            })
          }
        />
      )}
      <div className="relative flex min-h-0 flex-1 flex-col">
        {!snapshot ? (
          <div className="flex flex-1 items-center justify-center">
            <Spinner />
          </div>
        ) : snapshot.placeholder ? (
          <Placeholder placeholder={snapshot.placeholder} />
        ) : snapshot.empty ? (
          <div className="flex flex-1 items-center justify-center">
            <EmptyState
              icon={snapshot.empty.action === "clearFilters" ? ListFilter : Film}
              title={snapshot.empty.title}
              detail={snapshot.empty.detail ?? undefined}
              action={
                snapshot.empty.action === "goToCreate" ? (
                  <Button variant="secondary" title="Go to Create" onClick={() => void navigate({ to: "/create" })} />
                ) : undefined
              }
            />
          </div>
        ) : (
          <VirtualGrid
            items={cards}
            itemKey={(card) => card.id}
            onColumns={setColumns}
            scrollTo={cursor}
            footer={history && <HistoryFooterView footer={history} count={cards.length} />}
            renderItem={(card) => (
              <LibraryCard
                card={card}
                isSelected={pick === null && selection.ids.has(card.id)}
                isPicked={pick?.picked.includes(card.id) ?? false}
                isCurrent={cursor === card.id}
                run={runOf(card.id)}
                handlers={handlers}
              />
            )}
          />
        )}
      </div>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menu.items}
          onClose={() => {
            setMenu(null);
          }}
        />
      )}
      {deliverySets && (
        <DeliverySetsSheet
          initial={deliverySets.selection}
          onClose={() => {
            setDeliverySets(null);
          }}
        />
      )}
      {removing && (
        <AlertDialog
          title={removing.title}
          message=""
          onCancel={() => {
            setRemoving(null);
          }}
        >
          <Button
            variant="secondary"
            title="Cancel"
            onClick={() => {
              setRemoving(null);
            }}
          />
          <Button
            variant="primary"
            destructive
            title="Remove from Library"
            onClick={() => {
              void commands.libraryRemove(removing.ids);
              setRemoving(null);
            }}
          />
        </AlertDialog>
      )}
      {deleting && (
        <DeleteDialog
          ids={deleting.ids}
          images={deleting.images}
          onDone={(deleted) => {
            setDeleting(null);
            if (deleted) setSelection(emptySelection);
          }}
        />
      )}
      {preview && (
        <Preview
          title={preview.title}
          url={preview.mediaUrl}
          isVideo={preview.kind === "video"}
          onClose={() => {
            setPreview(null);
          }}
        />
      )}
      {subtitlesFor !== null && (
        <SubtitlesSheet
          id={subtitlesFor.id}
          title={subtitlesFor.title}
          onClose={() => {
            setSubtitlesFor(null);
          }}
        />
      )}
      {upscaling !== null && (
        <OnDeviceSheet
          id={upscaling}
          kind="upscaleImage"
          onClose={() => {
            setUpscaling(null);
          }}
        />
      )}
      {finals !== null && (
        <RenderFinalsSheet
          ids={finals}
          onClose={() => {
            setFinals(null);
          }}
        />
      )}
      {roleQuestion !== null && (
        <AlertDialog
          title={roleQuestion.ask.title}
          message={roleQuestion.ask.detail}
          onCancel={() => {
            setRoleQuestion(null);
          }}
        >
          <Button
            variant="secondary"
            title="Cancel"
            onClick={() => {
              setRoleQuestion(null);
            }}
          />
          {roleQuestion.ask.options.map((option) => (
            <Button
              key={option.title}
              variant="primary"
              title={option.title}
              onClick={() => {
                setRoleQuestion(null);
                void makeVideo(roleQuestion.ids, false, option.choice);
              }}
            />
          ))}
        </AlertDialog>
      )}
      {details !== null && (
        <ImageDetailsDialog
          id={details}
          versionId={null}
          onClose={() => {
            setDetails(null);
          }}
        />
      )}
    </div>
  );
}

/** The kind tabs with their counts, between the origin switch and the filter bar. */
export function LibraryKindTabs({
  kind,
  counts,
  onChange,
}: {
  kind: Kind;
  counts: LibraryCounts;
  onChange: (kind: Kind) => void;
}) {
  return (
    <div className="flex gap-medium">
      {kinds.map((option) => {
        const isSelected = option.value === kind;
        return (
          <button
            key={option.value}
            type="button"
            aria-pressed={isSelected}
            onClick={() => {
              onChange(option.value);
            }}
            className={cx(
              "flex cursor-pointer gap-x-small rounded-small py-x-small text-body",
              isSelected ? "font-bold text-accent" : "font-semibold text-muted",
              focusRing,
            )}
          >
            {option.title}
            <span className="text-faint">{counts[option.value].toLocaleString()}</span>
          </button>
        );
      })}
    </div>
  );
}

/** The filter bar (`LibraryFilterBar`): the menus and toggles Rust says this kind shows, in its order. */
function FilterBar({
  snapshot,
  filter,
  setFilter,
}: {
  snapshot: LibrarySnapshot_Serialize;
  filter: Filter;
  setFilter: (change: Partial<Filter>) => void;
}) {
  const { options } = snapshot;
  const any = (title: string, choices: Option<string>[]): Option<string | null>[] => [
    { value: null, title },
    ...choices,
  ];
  const plain = (values: string[]) => values.map((value) => ({ value, title: value }));
  const menu = (title: string, value: string | null, choices: Option<string | null>[], set: (value: string | null) => void) => (
    <FilterMenu key={title} title={title} value={value} options={choices} onChange={set} />
  );
  return (
    <div className="flex flex-wrap items-center gap-small pb-large">
      {snapshot.fields.map((field) => {
        switch (field) {
          case "mode":
            return menu("Mode", filter.mode, any("Any mode", options.modes), (mode) => {
              setFilter({ mode: mode as Filter["mode"] });
            });
          case "model":
            return menu("Video model", filter.model, any("Any video model", options.models), (model) => {
              setFilter({ model });
            });
          case "resolution":
            return menu("Resolution", filter.resolution, any("Any resolution", options.resolutions), (resolution) => {
              setFilter({ resolution: resolution as Filter["resolution"] });
            });
          case "finish":
            return (
              <FilterMenu
                key={field}
                title="Finish"
                value={filter.finish}
                options={finishes}
                onChange={(finish) => {
                  setFilter({ finish });
                }}
              />
            );
          case "imageModel":
            return menu("Image model", filter.imageModel, any("Any image model", plain(options.imageModels)), (imageModel) => {
              setFilter({ imageModel });
            });
          case "imageAspectRatio":
            return menu("Aspect", filter.imageAspectRatio, any("Any aspect", plain(options.imageAspects)), (imageAspectRatio) => {
              setFilter({ imageAspectRatio });
            });
          case "starred":
            return (
              <FilterToggle
                key={field}
                title="Starred"
                icon={Star}
                isOn={filter.isStarredOnly}
                onChange={(isStarredOnly) => {
                  setFilter({ isStarredOnly });
                }}
              />
            );
          case "rejected":
            return (
              <FilterToggle
                key={field}
                title="Show rejected"
                icon={EyeOff}
                iconOn={Eye}
                isOn={filter.showsRejected}
                onChange={(showsRejected) => {
                  setFilter({ showsRejected });
                }}
              />
            );
          case "sort":
            return (
              <FilterMenu
                key={field}
                title="Sort"
                value={filter.sort}
                options={sorts}
                onChange={(sort) => {
                  setFilter({ sort });
                }}
              />
            );
        }
      })}
    </div>
  );
}

/** "3 selected" with the bulk actions over the grid while results are selected (`LibrarySelectionBar`). */
function SelectionBar({
  count,
  bulk,
  onDone,
  onCull,
  onNewDeliverySet,
  onStar,
  onShow,
  onCopy,
  onDelete,
}: {
  count: number;
  /** Rust's answer for the picked results (`LibraryBulkActions`). */
  bulk: BulkActions | undefined;
  onDone: () => void;
  onCull: (action: CullingAction) => void;
  onNewDeliverySet: () => void;
  onStar: () => void;
  onShow: () => void;
  onCopy: () => void;
  onDelete: () => void;
}) {
  const title = bulk?.title ?? selectionTitle(count);
  return (
    <SelectionBarFrame title={title} onDone={onDone}>
      <CullingButtons count={count} onCull={onCull} />
      <Button variant="secondary" icon={Package} title={NEW_SET_TITLE} onClick={onNewDeliverySet} />
      {bulk !== undefined && bulk.starrableIds.length > 0 && (
        <Button variant="secondary" icon={Star} title={bulk.starsAll ? "Star" : "Unstar"} onClick={onStar} />
      )}
      {bulk !== undefined && bulk.fileIds.length > 0 && (
        <>
          <Button variant="secondary" icon={Folder} title="Show in File Manager" onClick={onShow} />
          <Button variant="secondary" icon={Copy} title="Copy Videos" hint="Copy videos" onClick={onCopy} />
          <Button variant="secondary" icon={Share} title="Share…" hint={shareHint} onClick={onShow} />
        </>
      )}
      {bulk !== undefined && bulk.deletableIds.length > 0 && (
        <Button variant="secondary" icon={Trash2} destructive title="Delete from Library…" onClick={onDelete} />
      )}
    </SelectionBarFrame>
  );
}

/**
 * "2 images selected" with the bulk actions over the grid while images are selected (`LibraryImageSelectionBar`):
 * saved images star, account images like, and an image whose file is gone can only be removed from the library.
 */
function ImageSelectionBar({
  chosen: images,
  onDone,
  onCull,
  onNewDeliverySet,
  onMakeVideo,
  onCompare,
  onTrash,
  onRemove,
}: {
  chosen: LibraryCard_Serialize[];
  onDone: () => void;
  onCull: (action: CullingAction) => void;
  onNewDeliverySet: () => void;
  onMakeVideo: (ids: string[], asReferences: boolean) => void;
  onCompare: (ids: string[]) => void;
  onTrash: (ids: string[]) => void;
  onRemove: (ids: string[]) => void;
}) {
  const saved = images.filter((image) => !image.needsImport);
  const remote = images.filter((image) => image.needsImport);
  const available = saved.filter((image) => !image.isUnavailable);
  const missing = saved.filter((image) => image.isUnavailable);
  const ids = (list: LibraryCard_Serialize[]) => list.map((image) => image.id);
  const starsAll = saved.every((image) => image.isStarred);
  const likesAll = remote.every((image) => image.isStarred);
  return (
    <SelectionBarFrame title={`${images.length.toLocaleString()} images selected`} onDone={onDone}>
      <CullingButtons count={images.length} onCull={onCull} />
      {saved.length > 0 && (
        <Button
          variant="secondary"
          icon={starsAll ? StarOff : Star}
          title={starsAll ? "Unstar" : "Star"}
          onClick={() => void commands.librarySetStarred(ids(saved), !starsAll)}
        />
      )}
      {remote.length > 0 && (
        <Button
          variant="secondary"
          icon={likesAll ? StarOff : Star}
          title={likesAll ? "Unlike" : "Like"}
          onClick={() => void commands.librarySetStarred(ids(remote), !likesAll)}
        />
      )}
      {(available.length > 0 || remote.length > 0) && (
        <Button
          variant="secondary"
          icon={Film}
          title="Make Video"
          onClick={() => {
            onMakeVideo(ids(images.filter((image) => !missing.includes(image))), false);
          }}
        />
      )}
      <Button variant="secondary" icon={Package} title={NEW_SET_TITLE} onClick={onNewDeliverySet} />
      {available.length > 0 && (
        <>
          <Button
            variant="secondary"
            icon={ImagePlus}
            title="Use as References"
            onClick={() => {
              onMakeVideo(ids(available), true);
            }}
          />
          {images.length === 2 && (
            <Button
              variant="secondary"
              icon={Columns2}
              title="Compare"
              onClick={() => {
                onCompare(ids(images));
              }}
            />
          )}
          <Button
            variant="secondary"
            icon={Trash2}
            destructive
            title="Move to Trash"
            onClick={() => {
              onTrash(ids(available));
            }}
          />
        </>
      )}
      {missing.length > 0 && (
        <Button
          variant="secondary"
          icon={CircleMinus}
          destructive
          title="Remove from Library"
          onClick={() => {
            onRemove(ids(missing));
          }}
        />
      )}
    </SelectionBarFrame>
  );
}

/** A selection bar's title, its actions and Done, which clears the selection (Esc does too). */
function SelectionBarFrame({ title, onDone, children }: { title: string; onDone: () => void; children: ReactNode }) {
  return (
    <div role="toolbar" aria-label={title} className="flex flex-wrap items-center gap-small pb-large">
      <h2 className="m-0 mr-auto text-body font-semibold text-muted">{title}</h2>
      {children}
      <Button variant="secondary" title="Done" hint="Clear selection (Esc)" onClick={onDone} />
    </div>
  );
}

function Placeholder({ placeholder }: { placeholder: HistoryPlaceholder }) {
  const body = (() => {
    switch (placeholder.kind) {
      case "unavailable":
        return <EmptyState icon={UserRound} title={placeholder.title} detail={placeholder.detail} />;
      case "failed":
        return (
          <EmptyState
            icon={TriangleAlert}
            title={placeholder.title}
            detail={placeholder.message}
            action={
              <Button variant="secondary" title="Try Again" onClick={() => void commands.libraryLoadHistory(true)} />
            }
          />
        );
      case "loading":
        return <Spinner label="Loading your Higgsfield history…" />;
      case "empty":
        return <EmptyState icon={Film} title={placeholder.title} detail={placeholder.detail} />;
    }
  })();
  return <div className="flex flex-1 items-center justify-center">{body}</div>;
}

/** Under the account's grid: Try Again after a failure, Load More after empty pages, else the next page on scroll. */
function HistoryFooterView({
  footer,
  count,
}: {
  footer: NonNullable<LibrarySnapshot_Serialize["history"]>;
  count: number;
}) {
  if (footer.isWaiting && footer.failure === null) {
    return (
      <div className="flex justify-center pt-large">
        <Button variant="secondary" title="Load More" onClick={() => void commands.libraryLoadHistory(false)} />
      </div>
    );
  }
  if (footer.note !== null && footer.failure === null) {
    return <p className="m-0 pt-large text-center text-caption text-faint">{footer.note}</p>;
  }
  return (
    <PagedFeedFooter
      failure={footer.failure === null ? undefined : { title: footer.failure }}
      hasMore={footer.hasMore && !footer.isLoading}
      count={count}
      loadMore={() => commands.libraryLoadHistory(false).then(() => undefined)}
    />
  );
}

/** The bar while two results are picked to compare (`ComparePickBar`): Rust's prompt, Cancel and Compare. */
function ComparePickBar({ pick, onCancel, onCompare }: { pick: ComparePick; onCancel: () => void; onCompare: () => void }) {
  return (
    <div role="toolbar" aria-label="Compare" className="flex items-center gap-small pb-large">
      <p className="m-0 mr-auto text-body font-semibold text-muted">{pick.prompt}</p>
      <Button variant="secondary" title="Cancel" onClick={onCancel} />
      <Button variant="primary" icon={Columns2} title="Compare" disabled={!pick.isReady} onClick={onCompare} />
    </div>
  );
}
