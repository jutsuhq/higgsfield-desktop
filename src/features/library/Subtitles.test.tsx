import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type {
  SubtitleEditorView_Serialize,
  SubtitleSaveOutcome_Serialize,
  SubtitleSheetView_Serialize,
  Subtitles_Serialize,
} from "../../bindings";
import { renderRoute } from "../../test/renderRoute";
import { resultDetail } from "./libraryTesting";

const sheet: SubtitleSheetView_Serialize = {
  title: "Subtitles",
  versionNote: "For the version on screen.",
  soundNote: "Made from the video's sound.",
  price: "Free",
  languages: [
    { value: "en", title: "English" },
    { value: "de", title: "German" },
  ],
  language: "en",
  engines: [
    { value: "auto", title: "Auto" },
    { value: "built_in", title: "Whisper" },
  ],
  engine: "auto",
  modelNotice: "Downloads Whisper Small (190 MB) from Hugging Face once.",
  modelConsent: "Download the model",
  run: null,
  current: null,
  unavailable: null,
  failure: null,
};

const subtitles: Subtitles_Serialize = {
  transcript: { language: "en", model: "whisper", revision: "5359861", words: [] },
  cues: [
    { id: "a", start: 0, end: 1.5, text: "Hello there" },
    { id: "b", start: 1.5, end: 3, text: "General Kenobi" },
  ],
  createdAt: "2026-11-13T10:00:00Z",
};

const editor: SubtitleEditorView_Serialize = {
  materialId: "final",
  versionId: "v1",
  title: "Subtitles",
  summary: "2 subtitles · English",
  subtitles,
  cues: [
    { id: "a", time: "0:00.0 – 0:01.5", canMerge: true, canSplit: true },
    { id: "b", time: "0:01.5 – 0:03.0", canMerge: false, canSplit: true },
  ],
};

describe("Subtitles", () => {
  it("makes subtitles only once the model's download is agreed to", async () => {
    const { calls } = await renderRoute("/library/result/final", {
      library_detail: () => resultDetail("final"),
      subtitles_sheet: () => sheet,
      subtitles_start: () => "run",
    });
    fireEvent.click(await screen.findByRole("button", { name: "Subtitles…" }));
    const dialog = await screen.findByRole("dialog", { name: "Subtitles" });
    expect(within(dialog).getByText("rooftop · Claymation")).toBeInTheDocument();
    const make = within(dialog).getByRole("button", { name: /^Make Subtitles/ });
    expect(make).toBeDisabled();
    fireEvent.click(within(dialog).getByRole("checkbox"));
    fireEvent.click(make);
    await waitFor(() => {
      expect(calls.find((call) => call.command === "subtitles_start")?.args).toEqual({
        id: "final",
        language: "en",
        engine: "auto",
        agreesToModel: true,
      });
    });
  });

  it("shows a run's progress with its Cancel link", async () => {
    const { calls } = await renderRoute("/library/result/final", {
      library_detail: () => resultDetail("final"),
      subtitles_sheet: () => ({ ...sheet, run: { id: "run", progress: "Making subtitles · 42%", fraction: 0.42 } }),
    });
    const run = await screen.findByRole("status", { name: "Making subtitles · 42%" });
    expect(within(run).getByText("Making subtitles · 42%")).toBeInTheDocument();
    fireEvent.click(within(run).getByRole("button", { name: "Cancel" }));
    await waitFor(() => {
      expect(calls.find((call) => call.command === "subtitles_cancel")?.args).toEqual({ runId: "run" });
    });
  });

  it("edits a cue in place of the page and asks before replacing subtitles made meanwhile", async () => {
    let saves = 0;
    const { calls } = await renderRoute("/library/result/final", {
      library_detail: () => resultDetail("final"),
      subtitles_sheet: () => ({ ...sheet, current: "English · 2 subtitles" }),
      subtitles_editor: () => editor,
      subtitles_edit: () => ({ ...editor, cues: [editor.cues[0]] }),
      subtitles_save: (args): SubtitleSaveOutcome_Serialize => {
        saves += 1;
        return args.replace
          ? { kind: "saved" }
          : { kind: "changedMeanwhile", title: "New subtitles were made", message: "Replace them?" };
      },
    });
    fireEvent.click(await screen.findByRole("button", { name: "Edit Subtitles" }));
    const dialog = await screen.findByRole("region", { name: "Subtitles" });
    expect(screen.queryByRole("button", { name: "More" })).not.toBeInTheDocument();
    const [merge] = within(dialog).getAllByRole("button", { name: "Merge with Next" });
    if (merge) fireEvent.click(merge);
    await waitFor(() => {
      expect(within(dialog).getAllByRole("button", { name: "Split" })).toHaveLength(1);
    });
    expect(calls.find((call) => call.command === "subtitles_edit")?.args.edit).toEqual({
      kind: "mergeWithNext",
      id: "a",
    });

    fireEvent.click(within(dialog).getByRole("button", { name: "Done" }));
    fireEvent.click(await screen.findByRole("button", { name: "Replace with Mine" }));
    await waitFor(() => {
      expect(screen.queryByRole("region", { name: "Subtitles" })).not.toBeInTheDocument();
    });
    expect(await screen.findByRole("button", { name: "More" })).toBeInTheDocument();
    expect(saves).toBe(2);
  });
});
