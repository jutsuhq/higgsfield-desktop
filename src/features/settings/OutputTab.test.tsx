import { fireEvent, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { OutputSettings } from "../../bindings";
import type { CommandHandlers } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";
import { internalError } from "./settingsTesting";

const output: OutputSettings = {
  folder: "/home/me/Videos/Jutsu Studio",
  folderDisplay: "~/Videos/Jutsu Studio",
  isCustomFolder: false,
  resolution: "480p",
  draftResolution: "480p",
  resolutions: ["480p", "720p", "1080p"],
  maxConcurrency: 2,
  minConcurrency: 1,
  maxConcurrencyLimit: 40,
  concurrencyDetail: "Generating at once. By default 8 on an account, 2 with an API key",
  namePattern: "{date}_{mode}_{id}",
  templates: [
    { title: "Date, mode, ID (default)", pattern: "{date}_{mode}_{id}" },
    { title: "Date and time, mode, ID", pattern: "{date}_{time}_{mode}_{id}" },
  ],
  tokenHelp: "{date} — date the job was queued, 2026-03-07",
};

/** Answers the preview as Rust would for the two patterns the tests type. */
const preview = (args: Record<string, unknown>) => {
  const pattern = args.pattern as string;
  return pattern.includes("{time}")
    ? { example: "2026-11-13_12-00-00_restyle_1a2b3c4d.mp4", footer: "Tokens: {date}." }
    : pattern === "{oops}"
      ? {
          example: "2026-11-13_restyle_1a2b3c4d.mp4",
          footer: "Empty pattern or unknown token: the default is used. Tokens: {date}.",
        }
      : { example: "2026-11-13_restyle_1a2b3c4d.mp4", footer: "Tokens: {date}." };
};

async function openOutput(handlers: CommandHandlers = {}) {
  const rendered = await renderRoute("/settings", {
    output_settings: () => output,
    output_name_preview: preview,
    ...handlers,
  });
  fireEvent.click(screen.getByRole("button", { name: "Output" }));
  await screen.findByRole("button", { name: "Change…" });
  return rendered;
}

describe("Settings › Output", () => {
  it("shows the default folder and changes it", async () => {
    const { calls } = await openOutput();
    expect(screen.queryByRole("button", { name: "Use Default" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Change…" }));
    expect(screen.queryByRole("button", { name: "Show in File Manager" })).not.toBeInTheDocument();
    await waitFor(() => {
      expect(calls.map((call) => call.command)).toEqual(expect.arrayContaining(["choose_output_folder"]));
    });
  });

  it("goes back to the default folder from a custom one", async () => {
    const { calls } = await openOutput({
      output_settings: () => ({ ...output, isCustomFolder: true, folderDisplay: "/media/Renders" }),
    });
    fireEvent.click(screen.getByRole("button", { name: "Use Default" }));
    await waitFor(() => {
      expect(calls.map((call) => call.command)).toContain("reset_output_folder");
    });
  });

  it("writes the resolutions and the parallel jobs", async () => {
    const { calls } = await openOutput();
    fireEvent.click(screen.getAllByRole("button", { name: "720p" })[0] as HTMLElement);
    fireEvent.click(screen.getAllByRole("button", { name: "1080p" })[1] as HTMLElement);
    fireEvent.click(screen.getByRole("button", { name: "Increase Parallel jobs" }));
    await waitFor(() => {
      expect(calls).toContainEqual({
        command: "set_preferences",
        args: { patch: { "settings.maxConcurrency": 3 } },
      });
    });
    expect(calls).toContainEqual({
      command: "set_preferences",
      args: { patch: { "settings.defaultResolution": "720p" } },
    });
    expect(calls).toContainEqual({
      command: "set_preferences",
      args: { patch: { "settings.draftResolution": "1080p" } },
    });
  });

  it("names files from a template or a typed pattern, with a live example", async () => {
    const { calls } = await openOutput();
    expect(await screen.findByText("2026-11-13_restyle_1a2b3c4d.mp4")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Templates" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Date and time, mode, ID" }));
    expect(screen.getByRole("textbox", { name: "Pattern" })).toHaveValue("{date}_{time}_{mode}_{id}");
    expect(await screen.findByText("2026-11-13_12-00-00_restyle_1a2b3c4d.mp4")).toBeInTheDocument();
    expect(calls).toContainEqual({
      command: "set_preferences",
      args: { patch: { "settings.outputNamePattern": "{date}_{time}_{mode}_{id}" } },
    });

    fireEvent.change(screen.getByRole("textbox", { name: "Pattern" }), { target: { value: "{oops}" } });
    expect(
      await screen.findByText("Empty pattern or unknown token: the default is used. Tokens: {date}."),
    ).toBeInTheDocument();
    const previewCall = calls.find((call) => call.command === "output_name_preview");
    expect(previewCall?.args).toHaveProperty("utcOffsetMinutes");
  });

  it("shows why a folder can't be used", async () => {
    await openOutput({
      choose_output_folder: internalError("Can't use this folder: permission denied"),
    });
    fireEvent.click(screen.getByRole("button", { name: "Change…" }));
    expect(await screen.findByRole("alertdialog", { name: "Can't change this setting" })).toBeInTheDocument();
    expect(screen.getByText("Can't use this folder: permission denied")).toBeInTheDocument();
  });
});
