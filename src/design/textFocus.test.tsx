import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { isEditingText, useTextAwareShortcut } from "./textFocus";

function Host({ action }: { action: () => void }) {
  useTextAwareShortcut("a", action);
  return (
    <>
      <input aria-label="Prompt" />
      <input aria-label="Agree" type="checkbox" />
      <button>Other</button>
    </>
  );
}

describe("useTextAwareShortcut", () => {
  it("runs the action while no text is edited", () => {
    const action = vi.fn();
    render(<Host action={action} />);
    screen.getByRole("button").focus();
    expect(isEditingText()).toBe(false);
    fireEvent.keyDown(window, { key: "a", ctrlKey: true });
    expect(action).toHaveBeenCalledOnce();
  });

  it("hands the key to the text being edited", () => {
    const action = vi.fn();
    render(<Host action={action} />);
    screen.getByRole("textbox", { name: "Prompt" }).focus();
    expect(isEditingText()).toBe(true);
    fireEvent.keyDown(window, { key: "a", ctrlKey: true });
    expect(action).not.toHaveBeenCalled();
  });

  it("does not count a checkbox as text", () => {
    render(<Host action={vi.fn()} />);
    screen.getByRole("checkbox").focus();
    expect(isEditingText()).toBe(false);
  });
});
