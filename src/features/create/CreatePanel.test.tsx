import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { CreateSnapshot_Serialize, PickerView_Serialize } from "../../bindings";
import { runPageCommand } from "../../lib/shortcuts/pageCommands";
import { emptyCreate } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";

function editsOf(calls: { command: string; args: Record<string, unknown> }[]) {
  return calls.filter((call) => call.command === "create_edit").map((call) => call.args.edit);
}

function choice(value: string, title: string) {
  return { value, title, unavailableReason: null };
}

function picker(title: string, options: [string, string][], selected: string): PickerView_Serialize {
  return {
    title,
    options: options.map(([value, name]) => choice(value, name)),
    selected,
    detail: null,
    unavailableReason: null,
    hint: null,
  };
}

/** Seedance and Cinema Studio offered, Cinema Studio picked, no source: a clip made from the prompt. */
const withoutSource: CreateSnapshot_Serialize = {
  ...emptyCreate,
  tasks: [choice("motionControl", "Motion transfer"), choice("edit", "Edit")],
  models: [
    { id: "seedance25", title: "Seedance 2.5", detail: "$0.30 / s", unavailableReason: null },
    { id: "cinemaStudio4", title: "Cinema Studio 4.0", detail: "$0.40 / s", unavailableReason: null },
  ],
  model: "cinemaStudio4",
  sourceDetail: "Optional · 4–30 s",
  sourceHint: "Or describe the clip in the prompt",
  clipLength: { title: "Length", detail: "8 s", value: 8, min: 4, max: 15 },
  aspectRatio: picker(
    "Frame",
    [
      ["auto", "Auto"],
      ["16:9", "16:9"],
      ["9:16", "9:16"],
    ],
    "auto",
  ),
  resolution: picker(
    "Resolution",
    [
      ["480p", "480p"],
      ["720p", "720p"],
    ],
    "720p",
  ),
  bitrate: {
    ...picker(
      "Quality",
      [
        ["standard", "Standard"],
        ["high", "High bitrate"],
      ],
      "standard",
    ),
    hint: "Video bitrate",
  },
  cinema: {
    controls: [
      {
        ...picker(
          "Camera",
          [
            ["", "Auto"],
            ["dolly", "Dolly"],
          ],
          "",
        ),
        detail: "camera",
      },
    ],
    summary: "Auto",
    isSet: false,
    unavailableReason: null,
    hint: "Camera, lens, genre and more",
  },
  audio: {
    ...picker(
      "Audio",
      [
        ["original", "Generate"],
        ["mute", "Silent"],
      ],
      "original",
    ),
    detail: "Made by Cinema Studio 4.0",
  },
  variants: { value: 1, min: 1, max: 4, unavailableReason: null },
};

describe("Create's panel", () => {
  it("picks the model from a capsule menu and says what the change did to the images' roles", async () => {
    const note = "Seedance 2.5 takes up to 5 reference images: 2 images were removed.";
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => withoutSource,
      create_change_roles: () => ({ snapshot: withoutSource, note, canUndo: true }),
    });
    const picker = await screen.findByRole("button", { name: "Model" });
    expect(picker).toHaveTextContent("Cinema Studio 4.0");
    fireEvent.click(picker);
    // Each model with its price beside it, the selected one checked.
    const cinema = screen.getByRole("menuitemradio", { name: "Cinema Studio 4.0 · $0.40 / s" });
    expect(cinema).toHaveAttribute("aria-checked", "true");
    fireEvent.click(screen.getByRole("menuitemradio", { name: "Seedance 2.5 · $0.30 / s" }));
    await waitFor(() => {
      expect(calls.find((call) => call.command === "create_change_roles")?.args.edit).toEqual({
        kind: "setModel",
        model: "seedance25",
      });
    });
    expect(editsOf(calls)).toEqual([]);
    // The note sits under the mode picker until dismissed; Undo brings back what the change cut.
    expect(await screen.findByText(note)).toBeInTheDocument();
    expect(screen.getByText("Draft replaced")).toBeInTheDocument();
    const panel = screen.getByRole("complementary", { name: "Settings" });
    fireEvent.click(within(panel).getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByText(note)).toBeNull();
  });

  it("sends a change of task through the role check, and shows no note when nothing moved", async () => {
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => withoutSource,
      create_change_roles: () => ({ snapshot: withoutSource, note: null, canUndo: false }),
    });
    const task = await screen.findByLabelText("Task");
    fireEvent.click(within(task).getByRole("button", { name: "Edit" }));
    await waitFor(() => {
      expect(calls.find((call) => call.command === "create_change_roles")?.args.edit).toEqual({
        kind: "pickTask",
        mode: "edit",
      });
    });
    const panel = screen.getByRole("complementary", { name: "Settings" });
    expect(within(panel).queryByRole("button", { name: "Dismiss" })).toBeNull();
  });

  it("titles File › Generate as the main button, and leaves File › Open… to the window", async () => {
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => ({
        ...withoutSource,
        submit: {
          ...withoutSource.submit,
          blocker: null,
          generate: { title: "Generate 3", price: null, blocker: null },
        },
      }),
      opened_files_choose: () => null,
    });
    await waitFor(() => {
      const published = calls.filter((call) => call.command === "app_menu_publish").at(-1);
      expect(published?.args.commands).toEqual(
        expect.arrayContaining([{ command: "generate", title: "Generate 3" }]),
      );
    });
    runPageCommand("open");
    await waitFor(() => {
      expect(calls.some((call) => call.command === "opened_files_choose")).toBe(true);
    });
    expect(calls.some((call) => call.command === "create_choose_source")).toBe(false);
  });

  it("numbers the references four to a row, each moved left or right and removed", async () => {
    const image = (name: string) => ({ path: `/tmp/${name}.png`, name, thumbnail: null });
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => ({
        ...withoutSource,
        references: {
          ...emptyCreate.references,
          items: [image("fox"), image("owl"), image("cat")],
          countText: "3 / 8",
          roles: ["start", "end"],
        },
      }),
    });
    const owl = await screen.findByRole("listitem", { name: "Image 2: owl" });
    expect(within(owl).getByText("2")).toBeInTheDocument();
    expect(within(owl).queryByRole("button", { name: /Use as/ })).toBeNull();
    fireEvent.click(within(owl).getByRole("button", { name: "Move right" }));
    fireEvent.click(within(owl).getByRole("button", { name: "Remove" }));
    const fox = screen.getByRole("listitem", { name: "Image 1: fox" });
    expect(within(fox).getByRole("button", { name: "Move left" })).toBeDisabled();
    await waitFor(() => {
      expect(editsOf(calls)).toEqual([
        { kind: "moveReference", from: 1, to: 2 },
        { kind: "removeReference", index: 1 },
      ]);
    });
  });

  it("turns the references off while frames are used, with the reason under them", async () => {
    const reason = "Seedance starts from the frames, so references wait.";
    await renderRoute("/create", {
      create_snapshot: () => ({
        ...withoutSource,
        references: { ...emptyCreate.references, disabledReason: reason },
      }),
    });
    const add = await screen.findByRole("button", { name: /Add images/ });
    expect(add.closest("fieldset")).toBeDisabled();
    expect(screen.getByText(reason)).toBeInTheDocument();
  });

  it("hides the Model section, with its task and options, when one model is offered", async () => {
    await renderRoute("/create", {
      create_snapshot: () => ({ ...withoutSource, models: withoutSource.models.slice(0, 1) }),
    });
    await screen.findByRole("complementary", { name: "Settings" });
    expect(screen.queryByRole("button", { name: "Model" })).toBeNull();
    expect(screen.queryByLabelText("Task")).toBeNull();
    expect(screen.queryByRole("button", { name: /Direction/ })).toBeNull();
  });

  it("offers Add a video without a source", async () => {
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => withoutSource,
      create_choose_source: () => null,
    });
    const add = await screen.findByRole("button", { name: /Add a video/ });
    fireEvent.click(add);
    await waitFor(() => {
      expect(calls.some((call) => call.command === "create_choose_source")).toBe(true);
    });
  });

  it("removes the source with Remove video and asks before upscaling", async () => {
    const loaded: CreateSnapshot_Serialize = {
      ...emptyCreate,
      source: {
        name: "dance.mov",
        fileText: "186 MB · HEVC",
        sizeText: "640×360",
        summary: "00:00.0 – 00:12.0 · 12 s",
        duration: 12,
        pills: [],
        playback: { path: null, progress: null, error: null },
        thumbnails: [],
      },
      upscale: { title: "Too small for object swap", detail: "It needs 720p.", allows: false },
    };
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => loaded,
      create_edit: () => loaded,
    });
    const panel = await screen.findByRole("complementary", { name: "Settings" });
    expect(within(panel).queryByRole("button", { name: /Add a video/ })).toBeNull();
    fireEvent.click(within(panel).getByRole("switch", { name: "Upscale before upload" }));
    fireEvent.click(within(panel).getByRole("button", { name: "Remove video" }));
    await waitFor(() => {
      expect(editsOf(calls)).toEqual([{ kind: "setAllowsUpscale", allows: true }, { kind: "removeSource" }]);
    });
  });

  it("sets the length with a slider and the frame from a menu", async () => {
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => withoutSource,
      create_edit: () => withoutSource,
    });
    const length = await screen.findByRole("slider", { name: "Length" });
    fireEvent.change(length, { target: { value: "11" } });
    fireEvent.click(screen.getByRole("button", { name: "Frame" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: "9:16" }));
    await waitFor(() => {
      expect(editsOf(calls)).toEqual([
        { kind: "setLength", seconds: 11 },
        { kind: "setAspectRatio", ratio: "9:16" },
      ]);
    });
  });

  it("turns a model's own sound off with Silent", async () => {
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => withoutSource,
      create_edit: () => withoutSource,
    });
    const audio = await screen.findByLabelText("Audio");
    fireEvent.click(within(audio).getByRole("button", { name: "Silent" }));
    await waitFor(() => {
      expect(editsOf(calls)).toEqual([{ kind: "setAudio", audio: "mute" }]);
    });
  });

  it("keeps Cinema's controls in a closed Direction row on Auto", async () => {
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => withoutSource,
      create_edit: () => withoutSource,
    });
    const direction = await screen.findByRole("button", { name: "Direction, Auto" });
    expect(direction).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("button", { name: "Camera" })).toBeNull();
    fireEvent.click(direction);
    fireEvent.click(screen.getByRole("button", { name: "Camera" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: "Dolly" }));
    await waitFor(() => {
      expect(editsOf(calls)).toEqual([{ kind: "setCinema", control: "camera", value: "dolly" }]);
    });
  });
});
