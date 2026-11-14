import { fireEvent, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { SubtitleSettingsView_Serialize } from "../../bindings";
import { renderRoute } from "../../test/renderRoute";

const subtitles: SubtitleSettingsView_Serialize = {
  engines: [
    { value: "auto", title: "Auto" },
    { value: "built_in", title: "Whisper" },
  ],
  defaultEngine: "auto",
  autoNote: "Auto hears a language…",
  modelsFolder: "Speech models",
  modelName: "Whisper Small",
  modelSummary: "Hears 99 languages on this computer.",
  modelCredit: "Whisper speech model: MIT.",
  modelFacts: "190 MB download · 99 languages · MIT",
  modelLanguages: "Afrikaans, Albanian, and Yoruba",
  modelUrl: "https://huggingface.co/x",
  license: "MIT",
  licenseUrl: "https://opensource.org/licenses/MIT",
  modelState: { kind: "notDownloaded" },
  modelSize: null,
  isAvailable: true,
  failure: null,
};

async function openTab(settings: SubtitleSettingsView_Serialize) {
  const { calls } = await renderRoute("/settings", { subtitle_settings: () => settings });
  fireEvent.click(screen.getByRole("button", { name: "Subtitles" }));
  await screen.findByText("Speech models");
  return calls;
}

describe("Settings › Subtitles", () => {
  it("downloads the model and changes the default engine", async () => {
    const calls = await openTab(subtitles);
    fireEvent.click(screen.getByRole("button", { name: "Download" }));
    fireEvent.click(screen.getByRole("button", { name: "Whisper" }));
    await screen.findByText("Speech models");
    const sent = calls.map((call) => call.command);
    expect(sent).toContain("subtitles_download_model");
    expect(calls.find((call) => call.command === "subtitles_set_default_engine")?.args).toEqual({
      engine: "built_in",
    });
  });

  it("keeps Download off where this build can't hear speech", async () => {
    await openTab({ ...subtitles, isAvailable: false });
    expect(screen.getByRole("button", { name: "Download" })).toBeDisabled();
  });
});
