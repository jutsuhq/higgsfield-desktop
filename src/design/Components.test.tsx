import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { percentText, RenderProgress } from "./Components";

describe("RenderProgress", () => {
  it("reads progress as a percentage, rounded half up and clamped", () => {
    expect(percentText(0.425)).toBe("43%");
    const { rerender } = render(<RenderProgress value={0.42} />);
    const bar = screen.getByRole("progressbar", { name: "Progress" });
    expect(bar).toHaveAttribute("aria-valuetext", "42%");
    rerender(<RenderProgress value={3} />);
    expect(bar).toHaveAttribute("aria-valuenow", "100");
  });
});
