import { fireEvent, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { NotificationSettings } from "../../bindings";
import { emitEvent } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";

const allOn: NotificationSettings = { onDone: true, onProblems: true, playsSound: true, canPlaySound: true };

describe("Settings › Notifications", () => {
  it("writes each switch and follows the settings", async () => {
    let settings = allOn;
    const { calls } = await renderRoute("/settings", { notification_settings: () => settings });
    fireEvent.click(screen.getByRole("button", { name: "Notifications" }));
    const done = await screen.findByRole("switch", { name: "Done" });

    fireEvent.click(done);
    fireEvent.click(screen.getByRole("switch", { name: "Failed, rejected or paused" }));
    fireEvent.click(screen.getByRole("switch", { name: "Play sound" }));
    await waitFor(() => {
      expect(calls).toContainEqual({
        command: "set_preferences",
        args: { patch: { "settings.notificationSound": false } },
      });
    });
    expect(calls).toContainEqual({
      command: "set_preferences",
      args: { patch: { "settings.notifyOnFinish": false } },
    });
    expect(calls).toContainEqual({
      command: "set_preferences",
      args: { patch: { "settings.notifyOnProblems": false } },
    });

    // Rust says both are off now: the sound has nothing to play with.
    settings = { onDone: false, onProblems: false, playsSound: true, canPlaySound: false };
    await emitEvent("preferences-changed", { preferences: {} });
    await waitFor(() => {
      expect(screen.getByRole("switch", { name: "Play sound" })).toBeDisabled();
    });
    expect(screen.getByRole("switch", { name: "Done" })).toHaveAttribute("aria-checked", "false");
  });

  it("says first when the system blocks the notifications, with its settings page", async () => {
    const { calls } = await renderRoute("/settings", {
      notification_settings: () => allOn,
      notifications_blocked: () => ({
        label: "Turned off in system settings",
        detail: "The system doesn't show Jutsu Studio notifications.",
        openTitle: "Open System Settings",
      }),
    });
    fireEvent.click(screen.getByRole("button", { name: "Notifications" }));
    fireEvent.click(await screen.findByRole("button", { name: "Open System Settings" }));
    expect(calls.some((call) => call.command === "notifications_open_system_settings")).toBe(true);
  });
});
