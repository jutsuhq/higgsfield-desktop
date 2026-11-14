import { screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { renderRoute } from "../../test/renderRoute";

const widths = (natural: number, row: number) => {
  Object.defineProperty(HTMLElement.prototype, "scrollWidth", {
    configurable: true,
    get(this: HTMLElement) {
      return this.classList.contains("w-max") ? natural : 0;
    },
  });
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => row });
};

afterEach(() => {
  widths(0, 0);
});

describe("Settings tabs", () => {
  it("are a menu when the window is too narrow for them (ViewThatFits)", async () => {
    widths(900, 500);
    await renderRoute("/settings");
    await screen.findByRole("heading", { level: 1, name: "Settings" });
    const menus = screen
      .getAllByRole("button", { name: /^Settings/ })
      .filter((button) => button.hasAttribute("aria-haspopup"));
    expect(menus).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Notifications" })).not.toBeInTheDocument();
  });

  it("are a row of tabs when they fit", async () => {
    widths(400, 500);
    await renderRoute("/settings");
    expect(await screen.findByRole("button", { name: "Notifications" })).toBeInTheDocument();
  });
});
