import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type {
  CharacterSetView_Serialize,
  CreateSnapshot_Serialize,
  HiggsfieldReferencesView_Serialize,
  MotionLibraryView_Serialize,
  PresetsView_Serialize,
} from "../../bindings";
import { emptyCreate } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";

const styleID = "11111111-1111-4111-8111-111111111111";
const goneID = "22222222-2222-4222-8222-222222222222";

const presets: PresetsView_Serialize = {
  title: "Styles",
  detail: "1 / 12",
  notice: null,
  isLoaded: true,
  isRefreshing: false,
  query: "",
  failure: null,
  showsTitles: true,
  sections: [
    {
      kind: "selected",
      title: "Selected",
      items: [
        {
          id: goneID,
          name: "Unavailable style",
          previewUrl: null,
          previewVideoUrl: null,
          isFavorite: false,
          isAvailable: false,
        },
      ],
    },
    {
      kind: "all",
      title: "All",
      items: [
        {
          id: styleID,
          name: "Anime",
          previewUrl: null,
          previewVideoUrl: null,
          isFavorite: false,
          isAvailable: true,
        },
      ],
    },
  ],
  selection: [goneID],
};

const restyle: CreateSnapshot_Serialize = {
  ...emptyCreate,
  batch: { hint: "One range × up to 12 styles, side by side", isOn: true },
  presets,
};

function editsOf(calls: { command: string; args: Record<string, unknown> }[]) {
  return calls.filter((call) => call.command === "create_edit").map((call) => call.args.edit);
}

describe("Create galleries", () => {
  it("picks, stars and searches styles, and switches the batch", async () => {
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => restyle,
      create_edit: () => restyle,
      create_preset_query: () => restyle,
    });
    expect(await screen.findByText("Styles")).toBeInTheDocument();
    expect(screen.getByText("1 / 12")).toBeInTheDocument();
    // A selected style that left the catalog can still be taken out of the batch.
    expect(screen.getByRole("button", { name: "Unavailable style, unavailable" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "Anime" }));
    fireEvent.click(screen.getByRole("switch", { name: "Batch" }));
    await waitFor(() => {
      expect(editsOf(calls)).toEqual([
        { kind: "pickPreset", id: styleID },
        { kind: "setBatch", on: false },
      ]);
    });
    fireEvent.click(screen.getByRole("button", { name: "Add to favorites" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Search styles" }), { target: { value: "ani" } });
    await waitFor(() => {
      expect(calls.find((call) => call.command === "create_toggle_favorite_preset")?.args).toEqual({
        id: styleID,
      });
      expect(calls.find((call) => call.command === "create_preset_query")?.args).toEqual({ query: "ani" });
    });
  });

  it("starts from a Motion Library clip on an empty stage", async () => {
    const library: MotionLibraryView_Serialize = {
      shelves: [
        { value: "featured", title: "Higgsfield" },
        { value: "trending", title: "Trending" },
      ],
      shelf: "featured",
      items: [{ id: "clip-1", name: "Spin", thumbnailUrl: null, previewUrl: null, example: "Motion" }],
      showsSkeleton: false,
      isLoading: false,
      failure: null,
      hasMore: false,
      downloadingId: null,
    };
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => ({ ...emptyCreate, showsMotionLibrary: true }),
      create_motion_library: () => library,
    });
    expect(await screen.findByText("Start from a clip")).toBeInTheDocument();
    await waitFor(() => {
      expect(calls.find((call) => call.command === "create_load_motion_library")?.args).toEqual({
        shelf: "featured",
        more: false,
      });
    });
    fireEvent.click(screen.getByRole("button", { name: "Spin, use as source" }));
    fireEvent.click(screen.getByRole("button", { name: "Trending" }));
    await waitFor(() => {
      expect(calls.find((call) => call.command === "create_use_motion_clip")?.args).toEqual({ id: "clip-1" });
      expect(calls.filter((call) => call.command === "create_load_motion_library").at(-1)?.args).toEqual({
        shelf: "trending",
        more: false,
      });
    });
  });

  it("saves a batch character to Higgsfield under the name typed, or removes it", async () => {
    const character: CharacterSetView_Serialize = {
      title: "Character 1",
      detail: "2 images",
      images: [
        { path: "/tmp/a.png", name: "a.png", thumbnail: null },
        { path: "/tmp/b.png", name: "b.png", thumbnail: null },
      ],
      imageCount: 2,
      save: "available",
      saveExplanation: "Saves these 2 images as a character on your Higgsfield account.",
    };
    const batch: CreateSnapshot_Serialize = {
      ...emptyCreate,
      batch: { hint: "One clip × up to 8 characters", isOn: true },
      characterSets: {
        detail: "1 / 8",
        sets: [character, { ...character, title: "Character 2", save: "saved" }],
        notice: null,
      },
    };
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => batch,
      create_edit: () => batch,
      create_save_character_set: () => null,
    });
    expect(await screen.findByText("Characters")).toBeInTheDocument();
    expect(screen.queryByText("Frames")).not.toBeInTheDocument();
    expect(screen.getByText("Saved to Higgsfield")).toBeInTheDocument();
    expect(screen.getByText("1–8 images of one character")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "More for character 1" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Save to Higgsfield…" }));
    const dialog = screen.getByRole("alertdialog", { name: "Save to Higgsfield…" });
    const name = within(dialog).getByRole("textbox", { name: "Name" });
    expect(name).toHaveValue("Character 1");
    fireEvent.change(name, { target: { value: "Mia" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    fireEvent.click(screen.getByRole("button", { name: "Remove character 2" }));
    await waitFor(() => {
      expect(calls.find((call) => call.command === "create_save_character_set")?.args).toEqual({
        index: 0,
        name: "Mia",
      });
      expect(editsOf(calls)).toEqual([{ kind: "removeCharacterSet", index: 1 }]);
    });
  });

  it("adds uploads and an element's picked images from Higgsfield", async () => {
    const sheet: HiggsfieldReferencesView_Serialize = {
      sources: [
        { value: "uploads", title: "Uploads" },
        { value: "elements", title: "Elements" },
      ],
      limit: 2,
      uploads: {
        title: "Your uploads on Higgsfield",
        detail: "Images you uploaded before, newest first. Pick up to 2 to use as references.",
        tiles: [
          { id: "m1", url: "https://example.com/1.png" },
          { id: "m2", url: "https://example.com/2.png" },
          { id: "m3", url: "https://example.com/3.png" },
        ],
        failure: null,
        hasMore: false,
        isEmpty: false,
      },
      elements: {
        unavailableReason: null,
        detail: "Pick one to add its images as references, up to 2.",
        category: null,
        categories: [{ value: null, title: "All categories" }],
        tiles: [{ id: "e1", name: "Mia", thumbnailUrl: null, tag: "Character" }],
        failure: null,
        hasMore: false,
        isEmpty: false,
      },
      characters: null,
      adding: null,
    };
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => ({ ...emptyCreate, offersHiggsfieldReferences: true }),
      create_higgsfield_references: () => sheet,
      create_add_uploads: () => null,
      create_open_element: () => ({
        kind: "choose",
        name: "Mia",
        detail: "It has more images than fit. Pick up to 2.",
        images: ["https://example.com/a.png", "https://example.com/b.png", "https://example.com/c.png"],
        picked: [0, 1],
        limit: 2,
      }),
      create_add_element_images: () => ({ kind: "added" }),
    });
    fireEvent.click(await screen.findByRole("button", { name: "From Higgsfield" }));
    expect(await screen.findByText("Your uploads on Higgsfield")).toBeInTheDocument();
    const [first, second, third] = screen.getAllByRole("button", { name: "Uploaded image" }) as [
      HTMLElement,
      HTMLElement,
      HTMLElement,
    ];
    fireEvent.click(first);
    fireEvent.click(third);
    expect(first).toHaveAccessibleName("Uploaded image, picked 1");
    // Two fit; the third waits.
    expect(second).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Add 2 Images" }));
    await waitFor(() => {
      expect(calls.find((call) => call.command === "create_add_uploads")?.args).toEqual({
        ids: ["m1", "m3"],
      });
      expect(screen.queryByText("Your uploads on Higgsfield")).not.toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("button", { name: "From Higgsfield" }));
    fireEvent.click(await screen.findByRole("button", { name: "Elements" }));
    fireEvent.click(await screen.findByRole("button", { name: "Mia" }));
    expect(await screen.findByText("It has more images than fit. Pick up to 2.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Element image, picked 1" }));
    const [, last] = screen.getAllByRole("button", { name: "Element image" }) as [HTMLElement, HTMLElement];
    fireEvent.click(last);
    fireEvent.click(screen.getByRole("button", { name: "Add 2 Images" }));
    await waitFor(() => {
      expect(calls.find((call) => call.command === "create_add_element_images")?.args).toEqual({
        indices: [1, 2],
      });
    });
  });

  it("downloads a pasted link as the source and says why one can't be used", async () => {
    const { calls } = await renderRoute("/create", {
      create_link_entry: () => ({
        title: "Paste a link to a video",
        placeholder: "https://",
        explanation: "A direct link to the file, starting with https://. Jutsu Studio downloads it first.",
        text: "",
      }),
      create_download_link: () => ({
        kind: "failed",
        message: {
          title: "This link isn't a video",
          message: "Add images under references instead.",
          action: null,
        },
      }),
    });
    expect(await screen.findByText("Source video")).toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: "Paste URL…" })[0] as HTMLElement);
    const field = await screen.findByRole("textbox", { name: "Paste a link to a video" });
    fireEvent.change(field, { target: { value: "https://example.com/a.png" } });
    fireEvent.click(screen.getByRole("button", { name: "Download" }));
    expect(await screen.findByText("This link isn't a video")).toBeInTheDocument();
    expect(calls.find((call) => call.command === "create_link_entry")?.args).toEqual({
      kind: "video",
      clipboard: "",
    });
    expect(calls.find((call) => call.command === "create_download_link")?.args).toEqual({
      text: "https://example.com/a.png",
      expected: "video",
    });
  });

  it("shows the speech recording by name and length, replaced or removed", async () => {
    const voiced: CreateSnapshot_Serialize = {
      ...emptyCreate,
      speech: { detail: "12.5 s", name: "voice.m4a" },
    };
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => voiced,
      create_edit: () => ({ ...voiced, speech: { detail: null, name: null } }),
      create_choose_speech: () => null,
    });
    expect(await screen.findByText("voice.m4a")).toBeInTheDocument();
    expect(screen.getByText("12.5 s")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Replace" }));
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(await screen.findByText("A voice recording the characters say")).toBeInTheDocument();
    expect(editsOf(calls)).toEqual([{ kind: "removeSpeech" }]);
    expect(calls.filter((call) => call.command === "create_choose_speech")).toHaveLength(1);
  });
});
