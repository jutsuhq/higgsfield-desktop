import { useEffect, useRef, useSyncExternalStore } from "react";

import type { PageCommand, PageCommandState } from "../../bindings";
import { isEditingText } from "../../design/textFocus";

/**
 * The commands the pages on screen carry out (`@FocusedValue` on macOS): a page registers each with the handler its
 * keyboard shortcut runs, so the window's menu runs the same one. Of several registrations of a command the first
 * enabled one wins, as the first key listener does.
 */
interface Registration {
  command: PageCommand;
  handler: () => void;
  isEnabled: boolean;
  title: string | null;
}

const registrations: Registration[] = [];
const listeners = new Set<() => void>();
let published: PageCommandState[] = [];

function changed() {
  const next: PageCommandState[] = [];
  for (const registration of registrations) {
    if (!registration.isEnabled || next.some((state) => state.command === registration.command)) continue;
    next.push({ command: registration.command, title: registration.title });
  }
  if (JSON.stringify(next) === JSON.stringify(published)) return;
  published = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The commands that apply now, with their titles, as the menu shows them. */
export function usePublishedCommands(): PageCommandState[] {
  return useSyncExternalStore(subscribe, () => published);
}

/**
 * Runs `command` as its menu item does. Delete from Library with text being edited deletes the word before the cursor
 * instead, which its shortcut (Ctrl+Backspace) does there (`TextFocus.send` on macOS).
 */
export function runPageCommand(command: PageCommand): void {
  if (command === "deleteResult" && isEditingText()) {
    deleteWordBackward();
    return;
  }
  registrations.find((registration) => registration.command === command && registration.isEnabled)?.handler();
}

function deleteWordBackward() {
  const element = document.activeElement;
  if (!(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement)) return;
  const end = element.selectionEnd ?? element.value.length;
  let start = element.selectionStart ?? end;
  if (start === end) {
    const wordStart = element.value.slice(0, start).replace(/\S+\s*$/u, "").length;
    start = wordStart === start ? Math.max(start - 1, 0) : wordStart;
  }
  // setRangeText skips the value setter React watches, so the input event below reaches its onChange.
  element.setRangeText("", start, end, "end");
  element.dispatchEvent(new Event("input", { bubbles: true }));
}

/**
 * Registers `handler` for `command` while the component is mounted (nothing for `null`); the menu enables it while
 * `isEnabled` and shows `title` (computed in Rust) in place of its own.
 */
export function usePageCommand(
  command: PageCommand | null,
  handler: () => void,
  isEnabled = true,
  title: string | null = null,
): void {
  const latest = useRef(handler);
  useEffect(() => {
    latest.current = handler;
  });
  const registration = useRef<Registration | null>(null);

  useEffect(() => {
    if (command === null) return;
    const entry: Registration = {
      command,
      handler: () => {
        latest.current();
      },
      isEnabled: false,
      title: null,
    };
    registration.current = entry;
    registrations.push(entry);
    return () => {
      const index = registrations.indexOf(entry);
      if (index >= 0) registrations.splice(index, 1);
      registration.current = null;
      changed();
    };
  }, [command]);

  useEffect(() => {
    const entry = registration.current;
    if (!entry) return;
    entry.isEnabled = isEnabled;
    entry.title = title;
    changed();
  }, [command, isEnabled, title]);
}

/** Forgets every registration; for tests. */
export function resetPageCommands(): void {
  registrations.length = 0;
  published = [];
}
