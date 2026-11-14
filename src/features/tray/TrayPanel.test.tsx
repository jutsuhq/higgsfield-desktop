import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { TrayPanel_Serialize } from "../../bindings";
import { emitEvent } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";

const idle: TrayPanel_Serialize = {
  header: {
    connectionLine: "API key · Committed today: $4.20",
    warning: null,
    agents: { isAllowed: false, status: "Agents off", control: null },
  },
  today: null,
  needsYou: [],
  needsYouOverflow: null,
  running: [],
  runningOverflow: null,
  recent: [],
  isIdle: true,
};

const busy: TrayPanel_Serialize = {
  header: {
    connectionLine: "Pro · 12,500 credits",
    warning: "Low credits: 42 left",
    agents: { isAllowed: true, status: "claude-code connected", control: "pause" },
  },
  today: {
    spent: "$4.20 spent",
    limit: "of $10.00",
    reserved: "$1.00 reserved",
    remaining: "$4.80 left",
    spentFraction: 0.42,
    reservedFraction: 0.1,
    isUsedUp: false,
    cap: null,
  },
  needsYou: [
    {
      id: "held",
      title: "Dance at dusk",
      reason: "Over the agents' budget",
      target: { kind: "job", id: "held" },
      hint: "Show in the Queue",
    },
    {
      id: "item:upscale_on_mac",
      title: "Rooftop",
      reason: "An agent asks to download the upscale model",
      target: { kind: "sheet", sheet: "upscale_on_mac", item: "item" },
      hint: "Open the item's sheet",
    },
  ],
  needsYouOverflow: "and 2 more",
  running: [
    {
      id: "walk",
      kind: "job",
      title: "Walk cycle",
      detail: "Generating · 40%",
      progress: 0.4,
      estimate: null,
      cancel: { isEnabled: true, help: "Cancel" },
    },
  ],
  runningOverflow: null,
  recent: [{ id: "jump", title: "Jump", detail: "3 min ago", file: "/tmp/jump.mp4" }],
  isIdle: false,
};

describe("tray panel", () => {
  it("says nothing runs and opens the app, Settings and Quit", async () => {
    const { calls } = await renderRoute("/tray", { tray_panel: () => idle });
    expect(await screen.findByText("Nothing running")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Pause the agents' jobs" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Open Jutsu Studio" }));
    fireEvent.click(screen.getByRole("button", { name: "Settings…" }));
    fireEvent.click(screen.getByRole("button", { name: "Quit Jutsu Studio" }));
    await waitFor(() => {
      expect(calls).toContainEqual({ command: "tray_quit", args: {} });
    });
    expect(calls).toContainEqual({ command: "tray_open", args: { place: { kind: "app" } } });
    expect(calls).toContainEqual({ command: "tray_open", args: { place: { kind: "settings" } } });
  });

  it("lists what waits, runs and finished, and each row acts", async () => {
    const { calls } = await renderRoute("/tray", { tray_panel: () => busy });
    const needsYou = await screen.findByRole("region", { name: "Needs you" });

    fireEvent.click(within(needsYou).getByText("Dance at dusk"));
    const running = screen.getByRole("region", { name: "Running" });
    expect(within(running).getByRole("progressbar")).toHaveAttribute("aria-valuenow", "40");
    fireEvent.click(within(running).getByText("Walk cycle"));
    fireEvent.click(within(running).getByRole("button", { name: "Cancel Walk cycle" }));
    const recent = screen.getByRole("region", { name: "Recent" });
    fireEvent.click(within(recent).getByText("Jump"));
    fireEvent.click(within(recent).getByRole("button", { name: "Show Jump in File Manager" }));
    expect(screen.getByText("Low credits: 42 left")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Pause the agents' jobs" }));

    await waitFor(() => {
      expect(calls).toContainEqual({ command: "tray_agents", args: { control: "pause" } });
    });
    for (const call of [
      { command: "tray_open", args: { place: { kind: "attention", target: { kind: "job", id: "held" } } } },
      { command: "tray_open", args: { place: { kind: "queueJob", id: "walk" } } },
      { command: "tray_cancel", args: { id: "walk" } },
      { command: "tray_open", args: { place: { kind: "libraryJob", id: "jump" } } },
      { command: "tray_reveal", args: { file: "/tmp/jump.mp4" } },
    ]) {
      expect(calls).toContainEqual(call);
    }
  });

  it("turns agents off from the panel", async () => {
    const { calls } = await renderRoute("/tray", { tray_panel: () => busy });
    const allow = await screen.findByRole("switch", { name: "Allow agents" });
    expect(allow).toHaveAttribute("aria-checked", "true");
    fireEvent.click(allow);
    await waitFor(() => {
      expect(calls).toContainEqual({ command: "agents_set_allowed", args: { isAllowed: false } });
    });
  });

  it("asks again every second while it shows, and not while hidden", async () => {
    const { calls } = await renderRoute("/tray", { tray_panel: () => idle });
    await screen.findByText("Nothing running");
    const asked = () => calls.filter((call) => call.command === "tray_panel").length;
    const first = asked();
    await waitFor(
      () => {
        expect(asked()).toBeGreaterThan(first);
      },
      { timeout: 2000 },
    );
    act(() => {
      window.dispatchEvent(new Event("blur"));
    });
    const hidden = asked();
    await act(() => new Promise((resolve) => setTimeout(resolve, 1300)));
    expect(asked()).toBe(hidden);
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    await waitFor(() => {
      expect(asked()).toBeGreaterThan(hidden);
    });
  });

  it("asks again when the queue changes", async () => {
    let answer = idle;
    await renderRoute("/tray", { tray_panel: () => answer });
    await screen.findByText("Nothing running");
    answer = busy;
    await emitEvent("queue-changed", { activeCount: 1 });
    expect(await screen.findByText("Walk cycle")).toBeInTheDocument();
  });
});
