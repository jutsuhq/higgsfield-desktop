import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { ShortcutRecording, ShortcutRow_Serialize, ShortcutSettings_Serialize } from "../../bindings";
import { emitEvent } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";

const row = (
  action: ShortcutRow_Serialize["action"],
  title: string,
  shortcut: string | null,
  isChanged = false,
) => ({
  action,
  title,
  shortcut,
  isChanged,
  resetHint: `Back to ${action === "create" ? "Ctrl+1" : "Ctrl+2"}`,
});

const defaults: ShortcutSettings_Serialize = {
  rows: [row("create", "Show Create", "Ctrl+1"), row("queue", "Show Queue", "Ctrl+2")],
  canRestoreDefaults: false,
};

async function openShortcuts(record: (args: Record<string, unknown>) => ShortcutRecording) {
  let settings = defaults;
  const rendered = await renderRoute("/settings", {
    shortcut_settings: () => settings,
    record_shortcut: record,
  });
  fireEvent.click(screen.getByRole("button", { name: "Shortcuts" }));
  await screen.findByRole("button", { name: "Show Create shortcut" });
  const update = async (next: ShortcutSettings_Serialize) => {
    settings = next;
    await emitEvent("preferences-changed", { preferences: {} });
  };
  return { ...rendered, update };
}

const press = async (init: KeyboardEventInit) => {
  await act(async () => {
    fireEvent.keyDown(window, init);
    await Promise.resolve();
  });
};

describe("Settings › Shortcuts", () => {
  it("records a new shortcut without the page switching", async () => {
    const { calls, router, update } = await openShortcuts(() => ({ kind: "saved" }));
    const recorder = screen.getByRole("button", { name: "Show Create shortcut" });
    expect(recorder).toHaveTextContent("Ctrl+1");

    fireEvent.click(recorder);
    expect(recorder).toHaveTextContent("Type shortcut…");
    await press({ key: "2", code: "Digit2", ctrlKey: true });
    await waitFor(() => {
      expect(calls).toContainEqual({
        command: "record_shortcut",
        args: {
          action: "create",
          stroke: { key: "2", code: "Digit2", ctrl: true, alt: false, shift: false, meta: false },
        },
      });
    });
    expect(router.state.location.pathname).toBe("/settings");
    await waitFor(() => {
      expect(recorder).not.toHaveTextContent("Type shortcut…");
    });

    await update({
      rows: [row("create", "Show Create", "Ctrl+Shift+K", true), row("queue", "Show Queue", null)],
      canRestoreDefaults: true,
    });
    expect(await screen.findByText("Ctrl+Shift+K")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Show Queue shortcut" })).toHaveTextContent("None");
    fireEvent.click(screen.getByRole("button", { name: "Reset Show Create" }));
    fireEvent.click(screen.getByRole("button", { name: "Restore Defaults" }));
    await waitFor(() => {
      expect(calls.map((call) => call.command)).toEqual(
        expect.arrayContaining(["reset_shortcut", "restore_default_shortcuts"]),
      );
    });
    expect(calls).toContainEqual({ command: "reset_shortcut", args: { action: "create" } });
  });

  it("keeps recording past a refused or ignored key and says why", async () => {
    const answers: ShortcutRecording[] = [
      { kind: "ignored" },
      { kind: "refused", message: "Already used by Show Queue" },
      { kind: "cancel" },
    ];
    await openShortcuts(() => answers.shift() ?? { kind: "cancel" });
    expect(screen.getByRole("button", { name: "Restore Defaults" })).toBeDisabled();
    const recorder = screen.getByRole("button", { name: "Show Create shortcut" });
    fireEvent.click(recorder);
    await press({ key: "Control", code: "ControlLeft", ctrlKey: true });
    await press({ key: "2", code: "Digit2", ctrlKey: true });
    expect((await screen.findAllByText("Already used by Show Queue"))[0]).toBeInTheDocument();
    expect(recorder).toHaveTextContent("Type shortcut…");
    await press({ key: "Escape", code: "Escape" });
    await waitFor(() => {
      expect(recorder).toHaveTextContent("Ctrl+1");
    });
    expect(screen.queryAllByText("Already used by Show Queue")).toHaveLength(0);
  });
});
