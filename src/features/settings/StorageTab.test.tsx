import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { OldDrafts_Serialize, StorageSummary } from "../../bindings";
import type { CommandHandlers } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";
import { internalError } from "./settingsTesting";

const summary: StorageSummary = {
  results: "1.43 GB · 12 files",
  sources: "200 MB",
  cache: "30 KB",
  media: "Empty",
  speechModels: "Empty",
  cacheDetail: "200 MB · prepared sources, previews and downloaded inputs",
  isBusy: false,
  canClearCache: true,
};

const drafts = (days: number): OldDrafts_Serialize =>
  days === 7
    ? {
        days,
        dayOptions: [7, 30, 90],
        summary: "3 drafts · 1.43 GB",
        confirmation: {
          title: "Move 3 drafts to the Trash?",
          message: "3 drafts older than 7 days, 1.43 GB in total.",
        },
        jobIds: ["a", "b", "c"],
      }
    : { days, dayOptions: [7, 30, 90], summary: "None", confirmation: null, jobIds: [] };

async function openStorage(handlers: CommandHandlers = {}) {
  const rendered = await renderRoute("/settings", {
    storage_summary: () => summary,
    old_drafts: (args) => drafts((args.days as number | null) ?? 30),
    ...handlers,
  });
  fireEvent.click(screen.getByRole("button", { name: "Storage" }));
  await screen.findByText("1.43 GB · 12 files");
  return rendered;
}

describe("Settings › Storage", () => {
  it("clears the cache after confirming and measures again", async () => {
    const { calls } = await openStorage();
    fireEvent.click(screen.getByRole("button", { name: "Clear Cache" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Clear the cache?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Clear Cache" }));
    await waitFor(() => {
      expect(calls.map((call) => call.command)).toContain("clear_cache");
    });
    // Measured again once it is done.
    await waitFor(() => {
      expect(calls.filter((call) => call.command === "storage_summary")).toHaveLength(2);
    });
  });

  it("waits for running jobs before clearing", async () => {
    await openStorage({ storage_summary: () => ({ ...summary, isBusy: true, canClearCache: false }) });
    expect(screen.getByRole("button", { name: "Clear Cache" })).toBeDisabled();
  });

  it("moves exactly the counted old drafts to the trash", async () => {
    const { calls } = await openStorage();
    expect((await screen.findAllByText("None"))[0]).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Move to Trash…" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "7 days" }));
    expect((await screen.findAllByText("3 drafts · 1.43 GB"))[0]).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Move to Trash…" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Move 3 drafts to the Trash?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Move to Trash" }));
    await waitFor(() => {
      expect(calls).toContainEqual({
        command: "trash_old_drafts",
        args: { days: 7, jobIds: ["a", "b", "c"] },
      });
    });
  });

  it("says why the space couldn't be freed", async () => {
    await openStorage({
      clear_cache: internalError("Permission denied"),
    });
    fireEvent.click(screen.getByRole("button", { name: "Clear Cache" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Clear the cache?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Clear Cache" }));
    expect(await screen.findByRole("alertdialog", { name: "Can't free the space" })).toBeInTheDocument();
    expect(screen.getByText("Permission denied")).toBeInTheDocument();
  });
});
