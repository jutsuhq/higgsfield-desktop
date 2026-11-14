import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { LibrarySnapshot_Serialize } from "../../bindings";
import { emitEvent, emptyLibrary } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";
import { defaultFilter, useLibraryView } from "./libraryData";
import { card, emptyMenu } from "./libraryTesting";

const grid: LibrarySnapshot_Serialize = {
  ...emptyLibrary,
  cards: [card("a", "Rooftop"), card("b", "Beach"), card("c", "Forest", { kind: "image", durationText: null })],
  counts: { videos: 2, images: 1, all: 3 },
  fields: ["starred", "rejected", "sort"],
  isOriginEmpty: false,
  empty: null,
};

const commandsFor = (snapshot: LibrarySnapshot_Serialize = grid) => ({
  library_snapshot: () => snapshot,
  library_thumbnail: () => null,
  library_cull: (args: Record<string, unknown>) => ({ action: args.action, cursor: args.cursor === null ? null : "c" }),
  library_undo_culling: () => "a",
  library_bulk: (args: Record<string, unknown>) => ({
    title: `${String((args.ids as string[]).length)} selected`,
    deletableIds: args.ids,
    starrableIds: args.ids,
    starsAll: true,
    starTitle: "Star",
    renderFinalId: null,
    compareIds: [],
    fileIds: args.ids,
  }),
  library_delete_confirmation: (args: Record<string, unknown>) =>
    args.images
      ? { title: "Move 1 images to the Trash?", message: "Their earlier versions…", confirmTitle: "Move to Trash" }
      : { title: "Delete 2 results from library?", message: "The video files…", confirmTitle: "Delete" },
});

afterEach(() => {
  useLibraryView.setState({ origin: "local", kind: "all", filter: defaultFilter, expandedGroups: [], order: [] });
});

describe("Library grid", () => {
  it("shows the cards with their tags and the counts of each tab", async () => {
    await renderRoute("/library", commandsFor());
    const rooftop = await screen.findByRole("button", { name: "Rooftop" });
    expect(within(rooftop).getByText("Motion")).toBeInTheDocument();
    expect(within(rooftop).getByText("Draft")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Videos/ })).toHaveTextContent("Videos2");
    expect(screen.getByRole("button", { name: "Show rejected" })).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Search prompts and names" })).toBeInTheDocument();
  });

  it("asks Rust again for another origin, kind or filter", async () => {
    const { calls } = await renderRoute("/library", commandsFor());
    await screen.findByRole("button", { name: "Rooftop" });
    fireEvent.click(screen.getByRole("button", { name: "From Higgsfield" }));
    fireEvent.click(screen.getByRole("button", { name: "Starred" }));
    await waitFor(() => {
      const requests = calls.filter((call) => call.command === "library_snapshot").map((call) => call.args.request);
      expect(requests).toContainEqual(
        expect.objectContaining({ origin: "higgsfield", filter: expect.objectContaining({ isStarredOnly: true }) as unknown }),
      );
    });
  });

  it("keeps, rejects and undoes with the stored keys on the current card", async () => {
    const { calls } = await renderRoute("/library", commandsFor());
    await screen.findByRole("button", { name: "Rooftop" });
    fireEvent.keyDown(window, { key: "ArrowRight", code: "ArrowRight" });
    fireEvent.keyDown(window, { key: "ArrowRight", code: "ArrowRight" });
    fireEvent.keyDown(window, { key: "x", code: "KeyX" });
    fireEvent.keyDown(window, { key: "z", code: "KeyZ", ctrlKey: true });
    await waitFor(() => {
      expect(calls.find((call) => call.command === "library_cull")?.args).toEqual({
        action: "reject",
        ids: ["b"],
        cursor: "b",
        order: ["a", "b", "c"],
        showsRejected: false,
      });
      expect(calls.some((call) => call.command === "library_undo_culling")).toBe(true);
    });
    // Undo makes the restored result the current card again.
    await waitFor(() => {
      expect(screen.getByRole("gridcell", { current: true })).toHaveAttribute("data-card", "a");
    });
  });

  it("selects with Ctrl-click, acts on the selection, and clears it with Esc", async () => {
    const { calls } = await renderRoute("/library", commandsFor());
    fireEvent.click(await screen.findByRole("button", { name: "Rooftop" }), { ctrlKey: true });
    fireEvent.click(screen.getByRole("button", { name: "Beach" }), { ctrlKey: true });
    const bar = screen.getByRole("toolbar", { name: "2 selected" });
    fireEvent.click(within(bar).getByRole("button", { name: "Keep 2 results" }));
    await waitFor(() => {
      expect(calls.find((call) => call.command === "library_cull")?.args).toMatchObject({ action: "keep", ids: ["a", "b"], cursor: null });
    });
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("toolbar")).not.toBeInTheDocument();
  });

  it("deletes the selection after the confirmation", async () => {
    const { calls } = await renderRoute("/library", commandsFor());
    fireEvent.click(await screen.findByRole("button", { name: "Rooftop" }), { ctrlKey: true });
    fireEvent.click(screen.getByRole("button", { name: "Beach" }), { shiftKey: true });
    fireEvent.click(await screen.findByRole("button", { name: "Delete from Library…" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Delete 2 results from library?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => {
      expect(calls.find((call) => call.command === "library_delete")?.args).toEqual({ ids: ["a", "b"] });
    });
  });

  it("tags only a draft, as `LibraryCards` does: a final has no tag", async () => {
    await renderRoute(
      "/library",
      commandsFor({ ...grid, cards: [card("a", "Rooftop"), card("f", "Final cut", { isDraft: false })] }),
    );
    const final = await screen.findByRole("button", { name: "Final cut" });
    expect(within(final).queryByText("Final")).not.toBeInTheDocument();
    expect(within(final).queryByText("Draft")).not.toBeInTheDocument();
  });

  it("folds a batch into one card that opens its comparison", async () => {
    const batch = card("batch-1", "dance · 2 styles", {
      batch: { count: 2, tag: "Batch × 2", memberIds: ["a", "b"], unstarredIds: ["b"], canRenderFinals: true },
    });
    const { router } = await renderRoute("/library", {
      ...commandsFor({ ...grid, cards: [batch] }),
      library_batch: () => {
        throw new Error("gone");
      },
    });
    const face = await screen.findByRole("button", { name: "dance · 2 styles" });
    expect(within(face).getByText("Batch × 2")).toBeInTheDocument();
    fireEvent.contextMenu(face);
    const items = within(await screen.findByRole("menu")).getAllByRole("menuitem").map((item) => item.textContent);
    expect(items).toEqual(["Select", "Share…", "Render Finals for Starred…", "Delete Unstarred…"]);
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.click(face);
    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/library/batch/batch-1");
    });
  });

  it("lists a draft's follow-ups in the Swift order", async () => {
    const draft = card("a", "Rooftop", {
      menu: { ...emptyMenu, canCompare: true, followUp: "renderFinal", canRerun: true, canEditVideo: true, canEditIn: true },
    });
    await renderRoute("/library", commandsFor({ ...grid, cards: [draft] }));
    fireEvent.contextMenu(await screen.findByRole("button", { name: "Rooftop" }));
    const items = within(await screen.findByRole("menu")).getAllByRole("menuitem").map((item) => item.textContent);
    expect(items).toEqual([
      "Star",
      "Compare…",
      "Show in File Manager",
      "Share…",
      "Export Short Preview…",
      "Copy Prompt",
      "Render Final…",
      "Edit & Re-run",
      "Edit…",
      "Extend…",
      "Subtitles…",
      "Edit in…",
      "Import as New Version…",
      "Select",
      "Delete from Library…",
      "Keep",
      "Reject",
      "Clear Keep or Reject",
    ]);
  });

  it("asks to move images to the Trash in their own words", async () => {
    const { calls } = await renderRoute("/library", commandsFor());
    fireEvent.contextMenu(await screen.findByRole("button", { name: "Forest" }));
    fireEvent.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Move to Trash…" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Move 1 images to the Trash?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Move to Trash" }));
    await waitFor(() => {
      expect(calls.find((call) => call.command === "library_delete")?.args).toEqual({ ids: ["c"] });
    });
  });

  it("offers an image made from an earlier version Compare with Original after Compare…", async () => {
    const edited = card("c", "Forest", { kind: "image", durationText: null, menu: { ...emptyMenu, canCompare: true, hasOriginal: true } });
    const { calls, router } = await renderRoute("/library", {
      ...commandsFor({ ...grid, cards: [edited] }),
      library_image_compare_original: () => {
        throw new Error("gone");
      },
    });
    fireEvent.contextMenu(await screen.findByRole("button", { name: "Forest" }));
    const items = within(await screen.findByRole("menu")).getAllByRole("menuitem").map((item) => item.textContent);
    expect(items.indexOf("Compare with Original")).toBe(items.indexOf("Compare…") + 1);
    fireEvent.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Compare with Original" }));
    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/library/compare-image-original/c");
      expect(calls.find((call) => call.command === "library_image_compare_original")?.args).toEqual({ id: "c" });
    });
  });

  it("asks Make Video's role when more than one fits, then places the image as picked", async () => {
    const endFrame = { roles: ["endFrame"] };
    const { calls, router } = await renderRoute("/library", {
      ...commandsFor(),
      library_make_video: (args: Record<string, unknown>) =>
        args.choice === null
          ? {
              kind: "ask",
              title: "How should Seedance 2.5 use this image?",
              detail: "A start or end frame makes the clip from the image alone, without a source video.",
              options: [
                { choice: { roles: ["startFrame"] }, title: "Start frame" },
                { choice: endFrame, title: "End frame" },
              ],
            }
          : { kind: "placed", canUndo: false, notice: null },
    });
    fireEvent.contextMenu(await screen.findByRole("button", { name: "Forest" }));
    fireEvent.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Make Video…" }));
    const question = await screen.findByRole("alertdialog", { name: "How should Seedance 2.5 use this image?" });
    fireEvent.click(within(question).getByRole("button", { name: "End frame" }));
    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/create");
    });
    const sent = calls.filter((call) => call.command === "library_make_video").map((call) => call.args.choice);
    expect(sent).toEqual([null, endFrame]);
  });

  it("opens a video's result page", async () => {
    const { router } = await renderRoute("/library", {
      ...commandsFor(),
      library_detail: () => {
        throw new Error("This result is no longer in the library.");
      },
    });
    fireEvent.click(await screen.findByRole("button", { name: "Rooftop" }));
    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/library/result/a");
    });
  });

  it("says nothing matches when the filters hide everything, as LibraryEmptyState does", async () => {
    await renderRoute(
      "/library",
      commandsFor({
        ...grid,
        cards: [],
        empty: { title: "Nothing matches these filters", detail: null, action: "clearFilters" },
      }),
    );
    expect(await screen.findByText("Nothing matches these filters")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Clear Filters" })).not.toBeInTheDocument();
  });
});

describe("From Higgsfield", () => {
  it("loads the first page, then refreshes with Ctrl+R and pages with Load More", async () => {
    useLibraryView.setState({ origin: "higgsfield" });
    const { calls } = await renderRoute(
      "/library",
      commandsFor({
        ...grid,
        cards: [card("h", "Remote", { needsImport: true, filePath: null })],
        history: { isLoading: false, hasMore: true, failure: null, isWaiting: true, canRefresh: true, note: null },
      }),
    );
    expect(await screen.findByRole("img", { name: "On Higgsfield, not downloaded yet" })).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "r", code: "KeyR", ctrlKey: true });
    fireEvent.click(screen.getByRole("button", { name: "Load More" }));
    fireEvent.contextMenu(screen.getByRole("button", { name: "Remote" }));
    fireEvent.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Download to Library" }));
    await waitFor(() => {
      const loads = calls.filter((call) => call.command === "library_load_history").map((call) => call.args.refresh);
      expect(loads).toEqual([true, true, false]);
      expect(calls.find((call) => call.command === "library_download")?.args).toEqual({ id: "h" });
    });
  });

  it("follows LibraryChanged", async () => {
    const { calls } = await renderRoute("/library", commandsFor());
    await screen.findByRole("button", { name: "Rooftop" });
    const before = calls.filter((call) => call.command === "library_snapshot").length;
    await emitEvent("library-changed", { revision: 1 });
    await waitFor(() => {
      expect(calls.filter((call) => call.command === "library_snapshot").length).toBeGreaterThan(before);
    });
  });
});

describe("Library search", () => {
  it("takes the focus on the Find shortcut", async () => {
    await renderRoute("/library", commandsFor());
    const field = await screen.findByRole("textbox", { name: "Search prompts and names" });
    await waitFor(() => {
      fireEvent.keyDown(window, { key: "f", code: "KeyF", ctrlKey: true });
      expect(field).toHaveFocus();
    });
  });
});

describe("Library Compare", () => {
  it("picks two results with Rust's prompts and opens them side by side", async () => {
    const { router } = await renderRoute("/library", {
      ...commandsFor(),
      library_compare_pick: (args: Record<string, unknown>) => {
        const picked = [...(args.picked as string[]), ...(args.toggled === null ? [] : [args.toggled as string])];
        const prompt = ["Pick two results to compare", "Pick one more", "Ready to compare"][picked.length] ?? "";
        return { picked, prompt, isReady: picked.length === 2 };
      },
    });
    fireEvent.click(await screen.findByRole("button", { name: "Compare" }));
    const bar = await screen.findByRole("toolbar", { name: "Compare" });
    expect(within(bar).getByText("Pick two results to compare")).toBeInTheDocument();
    expect(within(bar).getByRole("button", { name: "Compare" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Rooftop" }));
    expect(await within(bar).findByText("Pick one more")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Beach" }));
    expect(await within(bar).findByText("Ready to compare")).toBeInTheDocument();
    fireEvent.click(within(bar).getByRole("button", { name: "Compare" }));
    await waitFor(() => {
      expect(decodeURIComponent(router.state.location.pathname)).toBe("/library/compare/a,b");
    });
  });
});

describe("A selected card's menu", () => {
  it("offers the selection's actions, then Deselect All", async () => {
    await renderRoute("/library", commandsFor());
    const rooftop = await screen.findByRole("button", { name: "Rooftop" });
    fireEvent.click(rooftop, { ctrlKey: true });
    fireEvent.click(screen.getByRole("button", { name: "Beach" }), { ctrlKey: true });
    const bar = await screen.findByRole("toolbar", { name: "2 selected" });
    await within(bar).findByRole("button", { name: "Star" });
    fireEvent.contextMenu(rooftop);
    const items = within(await screen.findByRole("menu")).getAllByRole("menuitem").map((item) => item.textContent);
    expect(items).toEqual([
      "Star",
      "Show in File Manager",
      "Copy Videos",
      "Share…",
      "Delete from Library…",
      "Deselect All",
      "Keep 2 Results",
      "Reject 2 Results",
      "Clear Keep or Reject 2 Results",
    ]);
    fireEvent.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Deselect All" }));
    await waitFor(() => {
      expect(screen.queryByRole("toolbar", { name: "2 selected" })).not.toBeInTheDocument();
    });
  });
});

describe("Library image compare", () => {
  const imageMenu = { ...emptyMenu, canCompare: true, canCopyImage: true };
  const image = (id: string, title: string, overrides: Parameters<typeof card>[2] = {}) =>
    card(id, title, { kind: "image", durationText: null, modeTag: "Image", menu: imageMenu, ...overrides });
  const images: LibrarySnapshot_Serialize = {
    ...grid,
    cards: [card("a", "Rooftop"), image("c", "Forest"), image("d", "Lake"), image("m", "Missing", { isUnavailable: true, menu: emptyMenu })],
  };
  const picking = {
    library_compare_pick: (args: Record<string, unknown>) => {
      const before = args.picked as string[];
      const toggled = args.toggled as string | null;
      const picked =
        toggled === null ? before : before.includes(toggled) ? before.filter((id) => id !== toggled) : [...before, toggled].slice(-2);
      const prompt = ["Pick two results to compare", "Pick one more", "Ready to compare"][picked.length] ?? "";
      return { picked, prompt, isReady: picked.length === 2 };
    },
  };

  it("starts picking with the image Compare… was chosen on, among images only, and opens the two", async () => {
    const { router } = await renderRoute("/library", { ...commandsFor(images), ...picking });
    fireEvent.contextMenu(await screen.findByRole("button", { name: "Forest" }));
    fireEvent.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Compare…" }));
    const bar = await screen.findByRole("toolbar", { name: "Compare" });
    expect(await within(bar).findByText("Pick one more")).toBeInTheDocument();
    // A result and an image whose file is gone aren't picked among images.
    fireEvent.click(screen.getByRole("button", { name: "Rooftop" }));
    fireEvent.click(screen.getByRole("button", { name: "Missing" }));
    expect(within(bar).getByText("Pick one more")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Lake" }));
    expect(await within(bar).findByText("Ready to compare")).toBeInTheDocument();
    fireEvent.click(within(bar).getByRole("button", { name: "Compare" }));
    await waitFor(() => {
      expect(decodeURIComponent(router.state.location.pathname)).toBe("/library/compare-images/c,d");
    });
  });

  it("picks images from the header's Compare on the Images tab", async () => {
    useLibraryView.setState({ kind: "image" });
    const { calls } = await renderRoute("/library", { ...commandsFor(images), ...picking });
    fireEvent.click(await screen.findByRole("button", { name: "Compare" }));
    const bar = await screen.findByRole("toolbar", { name: "Compare" });
    fireEvent.click(screen.getByRole("button", { name: "Forest" }));
    expect(await within(bar).findByText("Pick one more")).toBeInTheDocument();
    expect(calls.some((call) => call.command === "library_download")).toBe(false);
  });

  it("compares two selected images from the selection bar", async () => {
    await renderRoute("/library", {
      ...commandsFor(images),
      ...picking,
      library_bulk: (args: Record<string, unknown>) => ({
        title: `${String((args.ids as string[]).length)} selected`,
        deletableIds: args.ids,
        starrableIds: args.ids,
        starsAll: true,
        starTitle: "Star",
        renderFinalId: null,
        compareIds: (args.ids as string[]).length === 2 ? args.ids : [],
        fileIds: [],
      }),
    });
    fireEvent.click(await screen.findByRole("button", { name: "Forest" }), { ctrlKey: true });
    fireEvent.click(screen.getByRole("button", { name: "Lake" }), { ctrlKey: true });
    const selection = await screen.findByRole("toolbar", { name: "2 images selected" });
    fireEvent.click(await within(selection).findByRole("button", { name: "Compare" }));
    const bar = await screen.findByRole("toolbar", { name: "Compare" });
    expect(await within(bar).findByText("Ready to compare")).toBeInTheDocument();
  });

  it("offers Copy Image for an image here, before Copy Prompt, and not for a missing one", async () => {
    const { calls } = await renderRoute("/library", commandsFor(images));
    fireEvent.contextMenu(await screen.findByRole("button", { name: "Forest" }));
    const items = within(await screen.findByRole("menu")).getAllByRole("menuitem").map((item) => item.textContent);
    expect(items.slice(items.indexOf("Details…"), items.indexOf("Details…") + 2)).toEqual(["Details…", "Compare…"]);
    expect(items.indexOf("Copy Image")).toBe(items.indexOf("Copy Prompt") - 1);
    fireEvent.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Copy Image" }));
    await waitFor(() => {
      expect(calls.find((call) => call.command === "library_copy_image")?.args).toEqual({ id: "c" });
    });
    fireEvent.contextMenu(screen.getByRole("button", { name: "Missing" }));
    const missing = within(await screen.findByRole("menu")).getAllByRole("menuitem").map((item) => item.textContent);
    expect(missing).not.toContain("Copy Image");
    expect(missing).not.toContain("Compare…");
  });

  it("opens Compare Variants of an image group two at a time, and of a video group as its batch", async () => {
    const grouped: LibrarySnapshot_Serialize = {
      ...grid,
      cards: [
        image("c", "Forest", { variantGroup: "images-1", variantCount: 3 }),
        card("a", "Rooftop", { variantGroup: "videos-1", variantCount: 2 }),
      ],
    };
    const { router } = await renderRoute("/library", commandsFor(grouped));
    fireEvent.contextMenu(await screen.findByRole("button", { name: "Forest" }));
    const items = within(await screen.findByRole("menu")).getAllByRole("menuitem").map((item) => item.textContent);
    expect(items.slice(items.indexOf("Show 3 Variants"), items.indexOf("Show 3 Variants") + 2)).toEqual([
      "Show 3 Variants",
      "Compare Variants",
    ]);
    fireEvent.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Compare Variants" }));
    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/library/compare-variants/images-1");
    });
    await router.navigate({ to: "/library" });
    fireEvent.contextMenu(await screen.findByRole("button", { name: "Rooftop" }));
    fireEvent.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Compare Variants" }));
    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/library/batch/videos-1");
    });
  });
});

describe("Library card marks", () => {
  it("shows a card's star, verdict, variants, aspect and the run on it", async () => {
    const marked = card("a", "Rooftop", {
      isStarred: true,
      culling: "kept",
      variantCount: 3,
      variantGroup: "group",
      kind: "image",
      durationText: null,
      aspectText: "16:9",
    });
    await renderRoute("/library", {
      ...commandsFor({ ...grid, cards: [marked, card("b", "Beach", { culling: "rejected" })] }),
      on_device_runs: () => [{ id: "run", itemId: "a", title: "Upscale", progress: "Upscaling · 40%", fraction: 0.4 }],
    });
    const rooftop = (await screen.findByRole("button", { name: "Rooftop" })).closest("[role=gridcell]");
    if (!(rooftop instanceof HTMLElement)) throw new Error("no cell");
    expect(within(rooftop).getByRole("img", { name: "Starred" })).toBeInTheDocument();
    expect(within(rooftop).getByRole("img", { name: "Kept" })).toBeInTheDocument();
    expect(within(rooftop).getByText("3 variants")).toBeInTheDocument();
    expect(within(rooftop).getByText("16:9")).toBeInTheDocument();
    expect(await within(rooftop).findByRole("status", { name: "Upscaling · 40%" })).toBeInTheDocument();
    const beach = screen.getByRole("button", { name: "Beach" }).closest("[role=gridcell]");
    expect(beach).toHaveClass("opacity-45");
  });

  it("offers Upscale on This Computer… on an image whose size allows it, before the verdicts", async () => {
    const forest = card("c", "Forest", {
      kind: "image",
      durationText: null,
      menu: { ...emptyMenu, canCompare: true, canCopyImage: true },
    });
    const { calls } = await renderRoute("/library", {
      ...commandsFor({ ...grid, cards: [forest] }),
      on_device_offers_image_upscale: () => true,
    });
    fireEvent.contextMenu(await screen.findByRole("button", { name: "Forest" }));
    const items = within(await screen.findByRole("menu")).getAllByRole("menuitem").map((item) => item.textContent);
    expect(items).toContain("Upscale on This Computer…");
    expect(items.slice(-3)).toEqual(["Keep", "Reject", "Clear Keep or Reject"]);
    expect(calls.find((call) => call.command === "on_device_offers_image_upscale")?.args).toEqual({ id: "c" });
  });
});
