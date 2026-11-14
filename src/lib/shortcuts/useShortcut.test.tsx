import { fireEvent, render, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { ShortcutBinding } from "../../bindings";
import { defaultShortcutBindings } from "./testing";
import { keyStroke, matchesBinding, useKeyBinding } from "./useShortcut";

const binding = (action: ShortcutBinding["action"]) => {
  const found = defaultShortcutBindings.find((each) => each.action === action);
  if (!found) throw new Error(`No default binding for ${action}`);
  return found;
};
const key = (init: KeyboardEventInit) => new KeyboardEvent("keydown", init);

describe("matchesBinding", () => {
  it("matches the physical key on any layout, and only with the same modifiers", () => {
    const create = binding("create");
    expect(matchesBinding(create, key({ key: "1", code: "Digit1", ctrlKey: true }))).toBe(true);
    expect(matchesBinding(create, key({ key: "&", code: "Digit1", ctrlKey: true }))).toBe(true);
    expect(matchesBinding(create, key({ key: "1", code: "Numpad1", ctrlKey: true }))).toBe(true);
    expect(matchesBinding(create, key({ key: "1", code: "Digit1" }))).toBe(false);
    expect(matchesBinding(create, key({ key: "1", code: "Digit1", ctrlKey: true, shiftKey: true }))).toBe(
      false,
    );
    expect(matchesBinding(create, key({ key: "1", code: "Digit1", ctrlKey: true, metaKey: true }))).toBe(
      false,
    );
    expect(matchesBinding(create, key({ key: "1", code: "Digit2", ctrlKey: true }))).toBe(false);
  });

  it("falls back to the key for an event without a code", () => {
    expect(matchesBinding(binding("retryJob"), key({ key: "R", ctrlKey: true, shiftKey: true }))).toBe(true);
  });

  it("turns a key press into what record_shortcut takes", () => {
    expect(keyStroke(key({ key: "k", code: "KeyK", ctrlKey: true, altKey: true }))).toEqual({
      key: "k",
      code: "KeyK",
      ctrl: true,
      alt: true,
      shift: false,
      meta: false,
    });
  });
});

describe("useKeyBinding", () => {
  it("runs on the shortcut, prevents its default, and stops when turned off", () => {
    const handler = vi.fn();
    const { rerender } = renderHook(
      ({ isEnabled }) => {
        useKeyBinding(binding("find"), handler, isEnabled);
      },
      { initialProps: { isEnabled: true } },
    );
    expect(fireEvent.keyDown(window, { key: "f", code: "KeyF", ctrlKey: true })).toBe(false);
    expect(handler).toHaveBeenCalledTimes(1);

    rerender({ isEnabled: false });
    fireEvent.keyDown(window, { key: "f", code: "KeyF", ctrlKey: true });
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it("leaves a text-aware key to the text being edited", () => {
    const handler = vi.fn();
    renderHook(() => {
      useKeyBinding(binding("deleteResult"), handler);
    });
    const { getByRole } = render(<input aria-label="Prompt" />);
    getByRole("textbox").focus();
    expect(fireEvent.keyDown(window, { key: "Backspace", code: "Backspace", ctrlKey: true })).toBe(true);
    expect(handler).not.toHaveBeenCalled();

    getByRole("textbox").blur();
    fireEvent.keyDown(window, { key: "Backspace", code: "Backspace", ctrlKey: true });
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it("does nothing until the binding arrives, or once it was removed", () => {
    const handler = vi.fn();
    renderHook(() => {
      useKeyBinding(undefined, handler);
    });
    fireEvent.keyDown(window, { key: "f", code: "KeyF", ctrlKey: true });
    expect(handler).not.toHaveBeenCalled();
  });
});
