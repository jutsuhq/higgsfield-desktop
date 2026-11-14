import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { PromptHistoryView_Serialize } from "../../bindings";
import { useToastStore } from "../../app/toastStore";
import { emptyCreate } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";

const withPrompt = { ...emptyCreate, prompt: { ...emptyCreate.prompt, text: "A fox in the snow" } };

const history: PromptHistoryView_Serialize = {
  rows: [
    { text: "A cat on a roof", lastUsed: null, isStarred: true },
    { text: "A fox in the snow", lastUsed: "2 hours ago", isStarred: false },
  ],
  emptyText: null,
};

afterEach(() => {
  useToastStore.setState({ queuedNotice: null, draftReplacement: null });
});

describe("Prompt history", () => {
  it("searches, stars and reuses a prompt with Undo", async () => {
    const { calls } = await renderRoute("/create", {
      prompts_history: () => history,
      create_replace_prompt: () => true,
    });
    fireEvent.click(await screen.findByRole("button", { name: "Prompt History" }));
    const picker = await screen.findByRole("dialog", { name: "Prompt History" });
    expect(await within(picker).findByText("2 hours ago")).toBeTruthy();

    fireEvent.change(within(picker).getByPlaceholderText("Search prompts"), { target: { value: "fox" } });
    await waitFor(() => {
      expect(calls.some((call) => call.command === "prompts_history" && call.args.query === "fox")).toBe(
        true,
      );
    });

    fireEvent.click(await within(picker).findByRole("button", { name: "Unstar" }));
    fireEvent.click(within(picker).getByRole("button", { name: "Star" }));
    fireEvent.click(within(picker).getByText("A cat on a roof"));
    await waitFor(() => {
      expect(useToastStore.getState().draftReplacement).not.toBeNull();
    });
    expect(screen.queryByRole("dialog", { name: "Prompt History" })).toBeNull();
    expect(calls.filter((call) => call.command === "prompts_set_starred").map((call) => call.args)).toEqual([
      { text: "A cat on a roof", starred: false },
      { text: "A fox in the snow", starred: true },
    ]);
    expect(calls.find((call) => call.command === "create_replace_prompt")?.args.text).toBe("A cat on a roof");
  });

  it("clears the video prompt through the replacement, with Undo", async () => {
    const { calls } = await renderRoute("/create", {
      create_snapshot: () => withPrompt,
      create_replace_prompt: () => true,
    });
    fireEvent.click(await screen.findByRole("button", { name: "Clear Prompt" }));
    await waitFor(() => {
      expect(useToastStore.getState().draftReplacement).not.toBeNull();
    });
    expect(calls.find((call) => call.command === "create_replace_prompt")?.args.text).toBe("");
  });
});
