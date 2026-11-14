import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import { ValueStepper } from "./ValueStepper";

function Stepper({ start }: { start: number }) {
  const [value, setValue] = useState(start);
  return <ValueStepper label="Variants" value={value} min={1} max={4} onChange={setValue} />;
}

describe("ValueStepper", () => {
  it("steps by one and stops at the ends of its range", () => {
    render(<Stepper start={1} />);
    const value = screen.getByRole("spinbutton", { name: "Variants" });
    const decrease = screen.getByRole("button", { name: "Decrease Variants" });
    const increase = screen.getByRole("button", { name: "Increase Variants" });
    expect(decrease).toBeDisabled();
    fireEvent.click(increase);
    fireEvent.click(increase);
    fireEvent.click(increase);
    expect(value).toHaveAttribute("aria-valuenow", "4");
    expect(increase).toBeDisabled();
    expect(decrease).toBeEnabled();
  });

  it("adjusts with the arrow keys inside the range", () => {
    render(<Stepper start={4} />);
    const value = screen.getByRole("spinbutton", { name: "Variants" });
    fireEvent.keyDown(value, { key: "ArrowUp" });
    expect(value).toHaveTextContent("4");
    fireEvent.keyDown(value, { key: "ArrowDown" });
    expect(value).toHaveTextContent("3");
  });
});
