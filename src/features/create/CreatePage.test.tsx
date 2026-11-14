import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { CreateSnapshot_Serialize, SubmitOutcome } from "../../bindings";
import { useToastStore } from "../../app/toastStore";
import { emitEvent, emptyCreate } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";
import { useCreateNotice } from "./createData";

/** A 47.3 s clip trimmed to 6–18.5 s, ready to generate at $8.85. */
const loaded: CreateSnapshot_Serialize = {
  ...emptyCreate,
  revision: 2,
  source: {
    name: "dance.mov",
    fileText: "186 MB · HEVC",
    sizeText: "1920×1080",
    summary: "00:06.0 – 00:18.5 · 12.5 s",
    duration: 47.3,
    pills: ["Converted for upload"],
    playback: { path: "/cache/create/abc/proxy.mp4", progress: null, error: null },
    thumbnails: ["/cache/create/abc/strip/0.png", "/cache/create/abc/strip/1.png"],
  },
  trim: {
    ranges: [{ start: 6, end: 18.5, text: "00:06.0 – 00:18.5" }],
    selected: 0,
    minLength: 4,
    maxLength: 30,
    uncovered: [],
    summary: "12.5 s selected · billed 13 s",
    canAddRange: true,
    isLocked: false,
    startValue: "00:06.0",
    endValue: "00:18.5",
  },
  issues: [],
  submit: {
    blocker: null,
    costLine: {
      text: "13 s × $0.681 = $8.85",
      detail: "List price · failed runs are free",
      isWarning: false,
    },
    generate: { title: "Generate", price: "$8.85", blocker: null },
    draft: { title: "Draft 480p", price: "$4.77", blocker: null },
    primaryIsDraft: false,
    isSubmitting: false,
  },
};

afterEach(() => {
  useCreateNotice.getState().dismiss();
  useToastStore.setState({ queuedNotice: null, draftReplacement: null });
});

describe("Create", () => {
  it("waits for a source video, with Generate off and why", async () => {
    await renderRoute("/create");
    expect(await screen.findByRole("button", { name: /Drop a video/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Generate" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Draft 480p" })).toBeDisabled();
  });

  it("plays a loaded source and nudges the trim with the keyboard", async () => {
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => loaded,
      media_url: ({ path }) => `http://media.test${String(path)}`,
      create_edit: () => loaded,
    });
    // Named on the stage and in the panel's Source video row.
    expect(await screen.findAllByText("dance.mov")).toHaveLength(2);
    await waitFor(() => {
      expect(screen.getByLabelText("dance.mov")).toHaveAttribute(
        "src",
        "http://media.test/cache/create/abc/proxy.mp4",
      );
    });
    fireEvent.keyDown(screen.getByRole("slider", { name: "Trim end" }), { key: "ArrowRight" });
    await waitFor(() => {
      expect(calls.find((call) => call.command === "create_edit")?.args).toEqual({
        edit: { kind: "nudge", part: "end", steps: 1 },
      });
    });
    // Shift moves by the same one step, as on macOS.
    fireEvent.keyDown(screen.getByRole("slider", { name: "Trim end" }), { key: "ArrowLeft", shiftKey: true });
    await waitFor(() => {
      expect(calls.filter((call) => call.command === "create_edit").at(-1)?.args).toEqual({
        edit: { kind: "nudge", part: "end", steps: -1 },
      });
    });
  });

  it("asks before going over today's cap, then queues approved", async () => {
    const confirm: SubmitOutcome = {
      kind: "confirm",
      confirmation: {
        title: "Go over today's limit?",
        message: "This job adds $8.85 and takes today to $28.85, over your $20.00 limit.",
        confirmTitle: "Generate anyway",
      },
    };
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => loaded,
      create_submit: ({ approved }) => (approved ? { kind: "queued", notice: "Queued · ~$8.85" } : confirm),
    });
    fireEvent.click(await screen.findByRole("button", { name: /Generate/ }));
    const dialog = await screen.findByRole("alertdialog", { name: "Go over today's limit?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Generate anyway" }));
    await waitFor(() => {
      expect(useToastStore.getState().queuedNotice?.text).toBe("Queued · ~$8.85");
    });
    expect(calls.filter((call) => call.command === "create_submit").map((call) => call.args)).toEqual([
      { isDraft: false, approved: false },
      { isDraft: false, approved: true },
    ]);
  });

  it("makes Draft the main button of a batch, which the shortcut presses", async () => {
    const batch = {
      ...loaded,
      submit: {
        ...loaded.submit,
        generate: { title: "Generate × 3", price: "$26.55", blocker: null },
        draft: { title: "Draft 480p × 3", price: "$14.31", blocker: null },
        primaryIsDraft: true,
      },
    };
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => batch,
      create_submit: () => ({ kind: "queued", notice: "3 drafts queued" }),
    });
    const buttons = await screen.findAllByRole("button", { name: /× 3/ });
    expect(buttons.map((button) => button.textContent)).toEqual([
      expect.stringContaining("Generate × 3"),
      expect.stringContaining("Draft 480p × 3"),
    ]);
    fireEvent.keyDown(window, { key: "Enter", code: "Enter", ctrlKey: true });
    await waitFor(() => {
      expect(calls.find((call) => call.command === "create_submit")?.args).toEqual({
        isDraft: true,
        approved: false,
      });
    });
  });

  it("asks for the form again when Rust says it changed", async () => {
    let answer = emptyCreate;
    await renderRoute("/create", { create_snapshot: () => answer });
    expect(await screen.findByRole("button", { name: /Drop a video/ })).toBeInTheDocument();
    answer = loaded;
    await emitEvent("create-changed", { revision: 2 });
    // Named on the stage and in the panel's Source video row.
    expect(await screen.findAllByText("dance.mov")).toHaveLength(2);
  });
});
