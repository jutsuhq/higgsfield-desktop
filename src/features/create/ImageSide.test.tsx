import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { ImageGenerateOutcome, ImageSnapshot_Serialize } from "../../bindings";
import { useCreateNotice } from "./createData";
import { runPageCommand } from "../../lib/shortcuts/pageCommands";
import { emptyImages } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";

const imageSide: ImageSnapshot_Serialize = {
  ...emptyImages,
  side: "image",
  prompt: "A red fox",
  generate: { isGenerating: false, enabled: true, price: "$0.0032", hint: "$0.0032 · Price from Higgsfield" },
  notice: null,
};

const withResults: ImageSnapshot_Serialize = {
  ...imageSide,
  images: [
    {
      id: "img-1",
      path: null,
      url: "https://example.com/1.png",
      prompt: "A red fox",
      model: "Soul 2",
      isPicked: false,
    },
    {
      id: "img-2",
      path: null,
      url: "https://example.com/2.png",
      prompt: "A blue fox",
      model: "Soul 2",
      isPicked: false,
    },
  ],
  current: {
    id: "img-1",
    path: null,
    url: "https://example.com/1.png",
    prompt: "A red fox",
    model: "Soul 2",
    isPicked: false,
  },
  runs: [
    {
      id: "run-1",
      title: "Soul 2 · 4 images",
      status: "working",
      detail: "Making the images…",
      canCancel: true,
      subtitle: null,
      review: null,
    },
  ],
  pendingTiles: 4,
};

afterEach(() => {
  useCreateNotice.getState().dismiss();
});

describe("Create's Image side", () => {
  it("switches sides through Rust", async () => {
    const { calls } = await renderRoute("/create");
    fireEvent.click(screen.getByRole("button", { name: "Image" }));
    expect(await screen.findByRole("button", { name: /Generate/ })).toBeTruthy();
    expect(calls.some((call) => call.command === "images_set_side" && call.args.side === "image")).toBe(true);
  });

  it("sends prompt and option edits, and clears the prompt", async () => {
    const { calls } = await renderRoute("/create", {
      images_snapshot: () => imageSide,
      images_edit: () => imageSide,
    });
    fireEvent.change(await screen.findByLabelText("Prompt"), { target: { value: "A red fox, snow" } });
    fireEvent.click(screen.getByRole("button", { name: "4" }));
    fireEvent.click(screen.getByRole("button", { name: "Clear Prompt" }));
    const edits = calls.filter((call) => call.command === "images_edit").map((call) => call.args.edit);
    expect(edits).toEqual([
      { kind: "setPrompt", text: "A red fox, snow" },
      { kind: "setCount", count: 4 },
      { kind: "setPrompt", text: "" },
    ]);
  });

  it("asks before generating over the cap, then generates approved", async () => {
    const answers: ImageGenerateOutcome[] = [
      {
        kind: "confirm",
        confirmation: {
          title: "Go over today's cap?",
          message: "This takes today's spending to $5.10 of your $5.00 cap.",
          confirmTitle: "Generate Anyway",
        },
      },
      { kind: "started" },
    ];
    const { calls } = await renderRoute("/create", {
      images_snapshot: () => imageSide,
      images_generate: () => answers.shift(),
    });
    fireEvent.click(await screen.findByRole("button", { name: /Generate/ }));
    const dialog = await screen.findByRole("alertdialog", { name: "Go over today's cap?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Generate Anyway" }));
    await waitFor(() => {
      expect(
        calls.filter((call) => call.command === "images_generate").map((call) => call.args.approved),
      ).toEqual([false, true]);
    });
  });

  it("says when it isn't connected", async () => {
    await renderRoute("/create", {
      images_snapshot: () => imageSide,
      images_generate: (): ImageGenerateOutcome => ({
        kind: "failed",
        notice: { title: "Not connected", detail: "Connect Higgsfield again to generate images." },
      }),
    });
    fireEvent.click(await screen.findByRole("button", { name: /Generate/ }));
    expect(await screen.findByRole("alertdialog", { name: "Not connected" })).toBeTruthy();
  });

  it("shows the results with the runs, picks, trashes and clears them", async () => {
    const { calls } = await renderRoute("/create", {
      images_snapshot: () => withResults,
      images_delete: () => ({ title: "Couldn't move the image to the Trash", detail: "The file is locked." }),
    });
    expect(await screen.findByText("Soul 2 · 4 images")).toBeTruthy();
    expect(screen.getAllByRole("img", { name: "Image being made" })).toHaveLength(4);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    fireEvent.click(screen.getByRole("button", { name: "A blue fox" }));
    fireEvent.click(screen.getAllByRole("button", { name: "Move to Trash" })[1] as HTMLElement);
    expect(
      await screen.findByRole("alertdialog", { name: "Couldn't move the image to the Trash" }),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "OK" }));
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    await waitFor(() => {
      expect(calls.map((call) => call.command)).toContain("images_clear");
    });
    const sent = calls.filter(
      (call) => call.command.startsWith("images_") && call.command !== "images_snapshot",
    );
    expect(sent.map((call) => [call.command, call.args.id])).toEqual([
      ["images_cancel", "run-1"],
      ["images_show", "img-2"],
      ["images_delete", "img-2"],
      ["images_clear", undefined],
    ]);
  });

  it("offers no Move to Trash on an image added to the references", async () => {
    const [first, second] = withResults.images;
    if (!first || !second) throw new Error("two images");
    await renderRoute("/create", {
      images_snapshot: () => ({ ...withResults, images: [{ ...first, isPicked: true }, second] }),
    });
    expect(await screen.findByRole("button", { name: "A blue fox" })).toBeTruthy();
    expect(screen.getAllByRole("button", { name: "Move to Trash" })).toHaveLength(1);
  });

  it("runs Generate from the shortcut and File › Generate", async () => {
    const { calls } = await renderRoute("/create", {
      images_snapshot: () => ({ ...imageSide, generate: { ...imageSide.generate, enabled: true } }),
      images_generate: (): ImageGenerateOutcome => ({ kind: "started" }),
    });
    await waitFor(() => {
      const published = calls.filter((call) => call.command === "app_menu_publish").at(-1);
      expect(published?.args.commands).toEqual(
        expect.arrayContaining([{ command: "generate", title: "Generate" }]),
      );
    });
    runPageCommand("generate");
    await waitFor(() => {
      expect(calls.some((call) => call.command === "images_generate")).toBe(true);
    });
  });

  it("shows a running request as a spinner with Cancel before its first image", async () => {
    const running = withResults.runs[0];
    const { calls } = await renderRoute("/create", {
      images_snapshot: () => ({ ...imageSide, running: running ?? null, runs: withResults.runs }),
    });
    const cancel = await screen.findByRole("button", { name: "Cancel" });
    expect(screen.queryByText("No images yet")).toBeNull();
    fireEvent.click(cancel);
    await waitFor(() => {
      expect(calls.some((call) => call.command === "images_cancel" && call.args.id === "run-1")).toBe(true);
    });
  });

  it("warns about unsaved records and copies the job IDs", async () => {
    const { calls } = await renderRoute("/create", {
      images_snapshot: () => ({
        ...imageSide,
        unsaved: {
          reason: "Can't write down image prices",
          detail: "Higgsfield accepted 2 jobs that would be lost on quit.",
          jobIds: ["a", "b"],
        },
      }),
    });
    fireEvent.click(await screen.findByRole("button", { name: "Try Again" }));
    await waitFor(() => {
      expect(calls.map((call) => call.command)).toContain("images_retry_saving");
    });
  });

  it("shows why there are no models instead of the form", async () => {
    await renderRoute("/create", {
      images_snapshot: () => ({
        ...imageSide,
        models: [],
        unavailableReason: "Connect Higgsfield to make images.",
      }),
    });
    expect(await screen.findByText("Connect Higgsfield to make images.")).toBeTruthy();
    expect(screen.queryByLabelText("Prompt")).toBeNull();
  });

  it("chooses and removes the image to edit", async () => {
    const editing = { ...imageSide, input: { detail: "Required", path: null, name: null } };
    const { calls } = await renderRoute("/create", {
      images_snapshot: () => editing,
      images_choose_input: () => ({
        ...editing,
        input: { detail: "Required", path: "/tmp/cat.png", name: "cat.png" },
      }),
      media_url: () => "asset://cat.png",
    });
    expect(await screen.findByText("Image to edit")).toBeTruthy();
    expect(screen.getByText("Required")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Choose…" }));
    fireEvent.click(await screen.findByRole("button", { name: "Remove" }));
    await waitFor(() => {
      expect(calls.some((call) => call.command === "images_edit")).toBe(true);
    });
    expect(calls.find((call) => call.command === "images_edit")?.args.edit).toEqual({
      kind: "setInput",
      path: null,
    });
  });
});
