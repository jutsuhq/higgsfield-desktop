import { fireEvent, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { disconnected } from "../lib/testing/ipcMock";
import { renderRoute } from "../test/renderRoute";

describe("Settings", () => {
  it("lists every tab but Spending while nothing is connected, alone with Back", async () => {
    await renderRoute("/settings", { connection_status: () => disconnected });
    expect(await screen.findByRole("button", { name: "Back" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connection" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByRole("button", { name: "Spending" })).not.toBeInTheDocument();
    expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(await screen.findByRole("heading", { level: 1, name: "Connect Higgsfield" })).toBeInTheDocument();
  });

  it("lists Spending once connected", async () => {
    await renderRoute("/settings");
    expect(await screen.findByRole("button", { name: "Spending" })).toBeInTheDocument();
  });
});
