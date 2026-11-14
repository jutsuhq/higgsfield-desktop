import { useEffect, useRef } from "react";

const textInputTypes = new Set(["text", "search", "url", "tel", "email", "password", "number", ""]);

/** Whether text is being edited: shortcuts that text editing uses too (Ctrl+A, Ctrl+V) belong to it then. */
export function isEditingText(element: Element | null = document.activeElement): boolean {
  if (!(element instanceof HTMLElement)) return false;
  if (element instanceof HTMLTextAreaElement) return !element.readOnly && !element.disabled;
  if (element instanceof HTMLInputElement) {
    return textInputTypes.has(element.type) && !element.readOnly && !element.disabled;
  }
  return element.closest('[contenteditable]:not([contenteditable="false"])') !== null;
}

/**
 * `TextAwareShortcut`: a window shortcut (Ctrl+`key`) that text editing also uses. While text is edited the key goes
 * to the text; otherwise `action` runs and the key's default is prevented.
 */
export function useTextAwareShortcut(key: string, action: () => void): void {
  const latest = useRef(action);
  useEffect(() => {
    latest.current = action;
  }, [action]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!event.ctrlKey || event.altKey || event.metaKey || event.key.toLowerCase() !== key.toLowerCase())
        return;
      if (isEditingText()) return;
      event.preventDefault();
      latest.current();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [key]);
}
