import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { GeneralSettings } from "../../bindings";
import type { CommandHandlers } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";
import { internalError } from "./settingsTesting";

const general: GeneralSettings = {
  defaultMode: "motionTransfer",
  modes: [
    { value: "motionTransfer", title: "Motion transfer" },
    { value: "restyle", title: "Restyle" },
    { value: "objectSwap", title: "Object swap" },
  ],
  showInTray: true,
  canChangeShowInTray: true,
  runInTray: false,
  showActiveCount: false,
  canShowActiveCount: true,
  openAtLogin: false,
  systemFooter: "Closing the window quits Jutsu Studio unless jobs are running.",
  versionText: "Version 0.1.0",
  lastCheckText: "Not checked yet",
  updatesEnabled: false,
  updatesFooter: "Updates are off in development builds.",
  checksAutomatically: true,
  downloadsAutomatically: false,
  canChangeDownloadsAutomatically: false,
  canCheckForUpdates: false,
};

const withUpdater: GeneralSettings = {
  ...general,
  updatesEnabled: true,
  updatesFooter: "",
  canChangeDownloadsAutomatically: true,
  canCheckForUpdates: true,
};

/** The settings row of the switch `name`. */
function row(name: string) {
  return screen.getByRole("switch", { name }).closest(".min-h-50");
}

async function openGeneral(handlers: CommandHandlers = {}) {
  const rendered = await renderRoute("/settings", { general_settings: () => general, ...handlers });
  fireEvent.click(screen.getByRole("button", { name: "General" }));
  await screen.findByText("Default mode");
  return rendered;
}

describe("Settings › General", () => {
  it("writes the switches and the mode", async () => {
    const { calls } = await openGeneral();
    // Updates off: the controls are disabled, not dimmed (`.disabled(!SoftwareUpdater.isEnabled)`).
    for (const name of ["Check automatically", "Download and install automatically"]) {
      expect(screen.getByRole("switch", { name })).toBeDisabled();
      expect(row(name)).not.toHaveClass("opacity-45");
    }
    expect(screen.getByRole("button", { name: "Check Now" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Restyle" }));
    fireEvent.click(screen.getByRole("switch", { name: "Run in the system tray" }));
    fireEvent.click(screen.getByRole("switch", { name: "Show in system tray" }));
    fireEvent.click(screen.getByRole("switch", { name: "Open at login" }));
    await waitFor(() => {
      expect(calls).toContainEqual({ command: "set_open_at_login", args: { isOn: true } });
    });
    for (const patch of [
      { "settings.defaultMode": "restyle" },
      { "settings.runInMenuBar": true },
      { "settings.showInMenuBar": false },
    ]) {
      expect(calls).toContainEqual({ command: "set_preferences", args: { patch } });
    }
  });

  it("dims the switches Rust locks", async () => {
    await openGeneral({
      general_settings: () => ({
        ...withUpdater,
        runInTray: true,
        canChangeShowInTray: false,
        showInTray: true,
        canShowActiveCount: false,
        checksAutomatically: false,
        canChangeDownloadsAutomatically: false,
      }),
    });
    const showInTray = screen.getByRole("switch", { name: "Show in system tray" });
    expect(showInTray).toBeDisabled();
    expect(showInTray).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("switch", { name: "Active job count" })).toBeDisabled();
    expect(screen.getByRole("switch", { name: "Check automatically" })).toBeEnabled();
    expect(screen.getByRole("switch", { name: "Download and install automatically" })).toBeDisabled();
    expect(row("Download and install automatically")).toHaveClass("opacity-45");
  });

  it("clears prompt history after confirming", async () => {
    const { calls } = await openGeneral();
    fireEvent.click(screen.getByRole("button", { name: "Clear History…" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Clear prompt history?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Clear History" }));
    await waitFor(() => {
      expect(calls.map((call) => call.command)).toContain("clear_prompt_history");
    });
  });

  it("writes the update switches and installs an update once agreed", async () => {
    const { calls } = await openGeneral({
      general_settings: () => withUpdater,
      check_for_updates: () => ({
        title: "A new version of Jutsu Studio is available!",
        message: "Jutsu Studio 0.2.0 is now available—you have 0.1.0.",
        installTitle: "Install Update",
      }),
      install_update: () => null,
    });
    const checks = screen.getByRole("switch", { name: "Check automatically" });
    expect(checks).toHaveAttribute("aria-checked", "true");
    fireEvent.click(checks);
    fireEvent.click(screen.getByRole("switch", { name: "Download and install automatically" }));
    await waitFor(() => {
      expect(calls).toContainEqual({
        command: "set_update_switch",
        args: { which: "downloadsAutomatically", isOn: true },
      });
    });
    expect(calls).toContainEqual({
      command: "set_update_switch",
      args: { which: "checksAutomatically", isOn: false },
    });

    fireEvent.click(screen.getByRole("button", { name: "Check Now" }));
    const dialog = await screen.findByRole("alertdialog", {
      name: "A new version of Jutsu Studio is available!",
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Install Update" }));
    await waitFor(() => {
      expect(calls.map((call) => call.command)).toContain("install_update");
    });
  });

  it("says updates aren't set up and shows a failed change", async () => {
    await openGeneral({
      general_settings: () => ({ ...general, canCheckForUpdates: true }),
      check_for_updates: () => ({
        title: "Can't check for updates",
        message: "Updates aren't set up in this build yet.",
      }),
      set_open_at_login: internalError("The autostart entry can't be written"),
    });
    fireEvent.click(screen.getByRole("button", { name: "Check Now" }));
    let dialog = await screen.findByRole("alertdialog", { name: "Can't check for updates" });
    expect(within(dialog).getByText("Updates aren't set up in this build yet.")).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "OK" }));

    fireEvent.click(screen.getByRole("switch", { name: "Open at login" }));
    dialog = await screen.findByRole("alertdialog", { name: "Can't change this setting" });
    expect(within(dialog).getByText("The autostart entry can't be written")).toBeInTheDocument();
  });
});
