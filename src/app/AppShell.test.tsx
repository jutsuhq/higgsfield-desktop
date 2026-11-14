import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { connectedWithKey, disconnected, emitConnection } from "../lib/testing/ipcMock";
import { renderRoute } from "../test/renderRoute";

const title = () => screen.getByRole("heading", { level: 1 }).textContent;

describe("AppShell", () => {
  it("opens Create from the root", async () => {
    const { router } = await renderRoute("/");
    expect(router.state.location.pathname).toBe("/create");
    expect(title()).toBe("Create");
  });

  it("switches pages with Ctrl+1, Ctrl+2, Ctrl+3 and opens Settings with Ctrl+,", async () => {
    await renderRoute("/create");
    for (const [key, page] of [
      ["2", "Queue"],
      ["3", "Library"],
      [",", "Settings"],
      ["1", "Create"],
    ] as const) {
      // The shortcuts arrive from Rust after the first render, so the key is pressed until the page opens.
      await waitFor(() => {
        fireEvent.keyDown(window, { key, ctrlKey: true });
        expect(screen.getByRole("heading", { level: 1, name: page })).toBeInTheDocument();
      });
    }
  });

  it("opens a page from its sidebar row", async () => {
    await renderRoute("/create");
    fireEvent.click(screen.getByRole("button", { name: "Library" }));
    expect(await screen.findByRole("heading", { level: 1, name: "Library" })).toBeInTheDocument();
  });

  it("steps through the screens with the arrow keys and shows the focus ring", async () => {
    await renderRoute("/create");
    const sidebar = screen.getByRole("navigation");
    act(() => {
      sidebar.focus();
    });
    fireEvent.keyDown(sidebar, { key: "ArrowUp" });
    expect(title()).toBe("Create");
    fireEvent.keyDown(sidebar, { key: "ArrowDown" });
    expect(await screen.findByRole("heading", { level: 1, name: "Queue" })).toBeInTheDocument();
    expect(screen.getByTestId("focus-ring")).toBeInTheDocument();
    for (let step = 0; step < 4; step++) fireEvent.keyDown(sidebar, { key: "ArrowDown" });
    expect(await screen.findByRole("heading", { level: 1, name: "Settings" })).toBeInTheDocument();
  });

  it("shows the connection's account plate and follows its changes", async () => {
    await renderRoute("/queue");
    const plate = await screen.findByRole("button", { name: "API key, billed in USD per job" });
    expect(plate).toBeInTheDocument();
    const lowCredits = {
      ...connectedWithKey,
      kind: "account" as const,
      plate: {
        badge: "Creator Plus",
        tier: "elite" as const,
        amount: "42",
        unit: "credits",
        warning: "Low credits: 42 left",
        isAPIKey: false,
        accessibilityLabel: "Creator Plus plan, 42 credits left",
      },
    };
    await emitConnection(lowCredits);
    expect(await screen.findByText("Low credits: 42 left")).toBeInTheDocument();
  });

  it("sends a window without a connection to onboarding, and a connected one back to Create", async () => {
    const { router } = await renderRoute("/queue", { connection_status: () => disconnected });
    expect(await screen.findByRole("heading", { level: 1, name: "Connect Higgsfield" })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/onboarding");
    await emitConnection(connectedWithKey);
    expect(await screen.findByRole("heading", { level: 1, name: "Create" })).toBeInTheDocument();
  });
});
