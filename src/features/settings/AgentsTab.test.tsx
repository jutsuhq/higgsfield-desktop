import { fireEvent, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { AgentSettingsView } from "../../bindings";
import { agentShowPath } from "../../app/agentShow";
import { emitEvent } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";

const off: AgentSettingsView = {
  isAllowed: false,
  status: "Ready for the jutsu helper",
  allowsUnlimited: false,
  centsText: "0.00",
  creditsText: "0",
  helperPath: "/opt/jutsu/jutsu",
  clients: [
    {
      name: "Claude Code",
      instructions: "Run this once in a terminal to add Jutsu Studio's tools to Claude Code.",
      snippet: 'claude mcp add jutsu -- "/opt/jutsu/jutsu" mcp',
      configFile: null,
    },
  ],
};

async function openAgents(settings: () => AgentSettingsView, extra: Record<string, unknown> = {}) {
  const rendered = await renderRoute("/settings", { agents_settings: settings, ...extra });
  fireEvent.click(screen.getByRole("button", { name: "Agents" }));
  await screen.findByRole("switch", { name: "Allow agents" });
  return rendered;
}

describe("Settings › Agents", () => {
  it("writes both switches and follows the settings", async () => {
    let settings = off;
    const { calls } = await openAgents(() => settings, {
      agents_set_allowed: () => null,
      agents_set_allows_unlimited: () => null,
    });
    fireEvent.click(screen.getByRole("switch", { name: "Allow agents" }));
    fireEvent.click(screen.getByRole("switch", { name: "Let agents use unlimited generations" }));
    await waitFor(() => {
      expect(calls).toContainEqual({ command: "agents_set_allowed", args: { isAllowed: true } });
    });
    expect(calls).toContainEqual({ command: "agents_set_allows_unlimited", args: { allows: true } });

    // Rust saved it: the tab follows the settings.
    settings = { ...off, isAllowed: true };
    await emitEvent("preferences-changed", { preferences: {} });
    await waitFor(() => {
      expect(screen.getByRole("switch", { name: "Allow agents" })).toHaveAttribute("aria-checked", "true");
    });
  });

  it("commits a budget and shows what Rust kept", async () => {
    const { calls } = await openAgents(() => off, { agents_commit_budget: () => "12.50" });
    const field = screen.getByRole("textbox", { name: "With an API key (USD)" });
    fireEvent.change(field, { target: { value: "$12.5" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() => {
      expect(field).toHaveValue("12.50");
    });
    expect(calls).toContainEqual({
      command: "agents_commit_budget",
      args: { connection: "apiKey", text: "$12.5" },
    });
  });
});

describe("show", () => {
  it("routes an agent's show to the page, the result or the item", () => {
    expect(agentShowPath({ page: "queue", job: "j1", item: null, sheet: null })).toBe("/queue");
    expect(agentShowPath({ page: "library", job: "j1", item: null, sheet: null })).toBe("/library/result/j1");
    expect(agentShowPath({ page: "library", job: null, item: "i1", sheet: "upscale" })).toBe(
      "/library/result/i1",
    );
    expect(agentShowPath({ page: "settings", job: null, item: null, sheet: null })).toBe("/settings");
  });
});
