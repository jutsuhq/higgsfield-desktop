import { useEffect, useRef } from "react";

import { commands, type KeyStroke, type PageCommand, type ShortcutAction, type ShortcutBinding } from "../../bindings";
import { isEditingText } from "../../design/textFocus";
import { useCommand } from "../ipc";
import { queryKeys } from "../queryKeys";
import { usePageCommand } from "./pageCommands";

/** The shortcuts set in Settings › Shortcuts, as Rust resolves them; removed ones are missing. */
export function useShortcutBindings() {
  return useCommand(queryKeys.settings.shortcutBindings, () => commands.shortcutBindings());
}

/** Ctrl+, opens Settings: fixed, as ⌘, is on macOS, so it is not in Settings › Shortcuts. */
export const settingsBinding: Omit<ShortcutBinding, "action"> = {
  codes: ["Comma"],
  key: ",",
  ctrl: true,
  alt: false,
  shift: false,
  yieldsToText: false,
};

/** Ctrl+Shift+S shows or hides the sidebar: View › Toggle Sidebar's fixed shortcut (⌃⌘S on macOS). */
export const toggleSidebarBinding: Omit<ShortcutBinding, "action"> = {
  codes: ["KeyS"],
  key: "s",
  ctrl: true,
  alt: false,
  shift: true,
  yieldsToText: false,
};

/** Ctrl+Q quits, as ⌘Q does on macOS; it asks first while work runs. */
export const quitBinding: Omit<ShortcutBinding, "action"> = {
  codes: ["KeyQ"],
  key: "q",
  ctrl: true,
  alt: false,
  shift: false,
  yieldsToText: false,
};

/** A key press as `record_shortcut` takes it. */
export function keyStroke(event: KeyboardEvent): KeyStroke {
  return {
    key: event.key,
    code: event.code,
    ctrl: event.ctrlKey,
    alt: event.altKey,
    shift: event.shiftKey,
    meta: event.metaKey,
  };
}

/**
 * Whether `event` presses `binding`: the same modifiers, and the same physical key (`code`, so another keyboard layout
 * still matches). An event without a code, such as a synthetic one, is matched by its key.
 */
export function matchesBinding(binding: Omit<ShortcutBinding, "action">, event: KeyboardEvent): boolean {
  if (
    event.metaKey ||
    event.ctrlKey !== binding.ctrl ||
    event.altKey !== binding.alt ||
    event.shiftKey !== binding.shift
  ) {
    return false;
  }
  if (binding.codes.length > 0 && event.code !== "") return binding.codes.includes(event.code);
  return event.key.toLowerCase() === binding.key;
}

/**
 * Runs `handler` when `binding` is pressed anywhere in the window and prevents the key's default. A binding that yields
 * to text (`TextAwareShortcut`) leaves the key to a text field being edited. Nothing runs while `binding` is
 * `undefined` or `isEnabled` is false.
 */
export function useKeyBinding(
  binding: Omit<ShortcutBinding, "action"> | undefined,
  handler: () => void,
  isEnabled = true,
): void {
  const latest = useRef(handler);
  useEffect(() => {
    latest.current = handler;
  });

  useEffect(() => {
    if (!binding || !isEnabled) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !matchesBinding(binding, event)) return;
      if (binding.yieldsToText && isEditingText()) return;
      event.preventDefault();
      latest.current();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [binding, isEnabled]);
}

/** The shortcuts whose command is also a menu item (`AppCommands.swift`), by the menu's name for it. */
const menuCommands: Partial<Record<ShortcutAction, PageCommand>> = {
  create: "create",
  queue: "queue",
  library: "library",
  open: "open",
  generate: "generate",
  star: "star",
  deleteResult: "deleteResult",
  cancelJob: "cancelJob",
  retryJob: "retryJob",
};

/**
 * `@StoredShortcut`: runs `handler` on the shortcut Settings › Shortcuts set for `action`, and never once it was removed.
 * An action with a menu item also runs it from there, titled `menuTitle` (computed in Rust) when given; the item is
 * enabled while `isEnabled`, with or without a shortcut.
 */
export function useShortcut(
  action: ShortcutAction,
  handler: () => void,
  isEnabled = true,
  menuTitle: string | null = null,
): void {
  const binding = useShortcutBindings().data?.find((candidate) => candidate.action === action);
  useKeyBinding(binding, handler, isEnabled);
  usePageCommand(menuCommands[action] ?? null, handler, isEnabled, menuTitle);
}
