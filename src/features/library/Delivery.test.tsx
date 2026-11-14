import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { DeliverySetView_Serialize, LibrarySnapshot_Serialize } from "../../bindings";
import { emptyLibrary } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";
import { defaultFilter, useLibraryView } from "./libraryData";
import { card } from "./libraryTesting";

const grid: LibrarySnapshot_Serialize = {
  ...emptyLibrary,
  cards: [card("a", "Rooftop"), card("b", "Beach")],
  counts: { videos: 2, images: 0, all: 2 },
  fields: [],
  isOriginEmpty: false,
  empty: null,
};

const choice = (value: string, title = value) => ({ value, title });

const set: DeliverySetView_Serialize = {
  id: "set-1",
  name: "Delivery Oct 13",
  status: "Not exported",
  isFrozen: false,
  lockHint: "Versions lock on first export",
  items: [
    {
      materialId: "a",
      versionId: "v-a",
      name: "Rooftop",
      isVideo: true,
      preview: { path: null, url: null, exists: false },
      frameAspects: [choice("4x5", "4:5")],
      state: { kind: "failed", message: "The disk is full." },
      stateHint: "The disk is full.",
    },
  ],
  outputs: [
    {
      id: "out-1",
      name: "",
      placeholder: "editing",
      summary: "Editing H.264 · MP4",
      isVideo: true,
      format: "jpeg",
      size: "full",
      quality: null,
      colorSpace: "srgb",
      background: null,
      videoPreset: "editingH264",
      subtitles: "none",
      aspect: "original",
    },
  ],
  options: {
    formats: [choice("jpeg", "JPEG")],
    sizes: [choice("full", "Full size")],
    qualities: [],
    colorSpaces: [choice("srgb", "sRGB")],
    videoPresets: [choice("editingH264", "Editing H.264"), choice("prores", "ProRes 422")],
    subtitles: [choice("none", "None")],
    aspects: [choice("original", "Original frame"), choice("4x5", "4:5")],
  },
  note: "",
  reviewLink: "",
  counts: null,
  failedCount: 0,
  pendingCount: 0,
  canExport: true,
  hasFolder: false,
  isExporting: false,
};

const commandsFor = (detail: DeliverySetView_Serialize = set) => ({
  library_snapshot: () => grid,
  library_thumbnail: () => null,
  library_bulk: (args: Record<string, unknown>) => ({
    title: `${String((args.ids as string[]).length)} selected`,
    deletableIds: [],
    starrableIds: [],
    starsAll: true,
    starTitle: "Star",
    renderFinalId: null,
    compareIds: [],
    fileIds: [],
  }),
  delivery_new: () => "set-1",
  delivery_sets: () => [{ id: "set-1", name: detail.name, status: detail.status }],
  delivery_set: () => detail,
  delivery_change: () => null,
});

afterEach(() => {
  useLibraryView.setState({ origin: "local", kind: "all", filter: defaultFilter, expandedGroups: [], order: [] });
});

describe("Delivery sets", () => {
  it("makes a set from the selection and opens it", async () => {
    const { calls } = await renderRoute("/library", commandsFor());
    fireEvent.click(await screen.findByRole("button", { name: "Rooftop" }), { ctrlKey: true });
    fireEvent.click(await screen.findByRole("button", { name: "New Delivery Set" }));
    const sheet = await screen.findByRole("dialog", { name: "Delivery Sets" });
    expect(calls).toContainEqual({ command: "delivery_new", args: { ids: ["a"] } });
    expect(await within(sheet).findByRole("textbox", { name: "Set name" })).toHaveValue("Delivery Oct 13");
    expect(within(sheet).getByRole("option", { name: /Delivery Oct 13/ })).toHaveAttribute("aria-selected", "true");
    expect(within(sheet).getByLabelText("The disk is full.")).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Frame 4:5…" })).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Export…" })).toBeEnabled();
  });

  it("saves each edit of a set", async () => {
    const { calls } = await renderRoute("/library", commandsFor());
    fireEvent.click(await screen.findByRole("button", { name: "Delivery Sets" }));
    const sheet = await screen.findByRole("dialog", { name: "Delivery Sets" });
    fireEvent.change(await within(sheet).findByRole("textbox", { name: "Note" }), { target: { value: "Round two" } });
    fireEvent.click(within(sheet).getByRole("button", { name: "Add Image Output" }));
    fireEvent.click(within(sheet).getByRole("button", { name: "Remove" }));
    await waitFor(() => {
      expect(calls).toContainEqual({
        command: "delivery_change",
        args: { id: "set-1", change: { kind: "removeItem", version_id: "v-a" } },
      });
    });
    expect(calls).toContainEqual({ command: "delivery_change", args: { id: "set-1", change: { kind: "note", note: "Round two" } } });
    expect(calls).toContainEqual({
      command: "delivery_change",
      args: { id: "set-1", change: { kind: "addOutput", is_video: false } },
    });
  });

  it("offers only the changes of an exported set and asks before deleting it", async () => {
    const frozen = { ...set, isFrozen: true, status: "Exported 13 Oct 2025, 12:00 to Client", failedCount: 1, pendingCount: 2, hasFolder: true };
    const { calls } = await renderRoute("/library", { ...commandsFor(frozen), delivery_delete: () => null });
    fireEvent.click(await screen.findByRole("button", { name: "Delivery Sets" }));
    const sheet = await screen.findByRole("dialog", { name: "Delivery Sets" });
    expect(await within(sheet).findByRole("button", { name: "Export 2 Pending" })).toBeEnabled();
    expect(within(sheet).getByRole("button", { name: "Retry 1 Failed" })).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Show in File Manager" })).toBeInTheDocument();
    expect(within(sheet).queryByRole("button", { name: "Remove" })).not.toBeInTheDocument();

    fireEvent.click(within(sheet).getByRole("button", { name: "Delete…" }));
    const confirm = await screen.findByRole("alertdialog", { name: "Delete “Delivery Oct 13”?" });
    fireEvent.click(within(confirm).getByRole("button", { name: "Delete Set" }));
    await waitFor(() => {
      expect(calls).toContainEqual({ command: "delivery_delete", args: { id: "set-1" } });
    });
  });
});
