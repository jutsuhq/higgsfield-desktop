import type { ShortcutBinding } from "../../bindings";

const ctrl = { ctrl: true, alt: false, shift: false, yieldsToText: false };

/** What `shortcut_bindings` answers with nothing changed in Settings › Shortcuts, for tests. */
export const defaultShortcutBindings: ShortcutBinding[] = [
  { action: "create", codes: ["Digit1", "Numpad1"], key: "1", ...ctrl },
  { action: "queue", codes: ["Digit2", "Numpad2"], key: "2", ...ctrl },
  { action: "library", codes: ["Digit3", "Numpad3"], key: "3", ...ctrl },
  { action: "open", codes: ["KeyO"], key: "o", ...ctrl },
  { action: "generate", codes: ["Enter", "NumpadEnter"], key: "enter", ...ctrl },
  { action: "find", codes: ["KeyF"], key: "f", ...ctrl },
  { action: "refreshHistory", codes: ["KeyR"], key: "r", ...ctrl },
  { action: "star", codes: ["KeyL"], key: "l", ...ctrl },
  { action: "deleteResult", codes: ["Backspace"], key: "backspace", ...ctrl, yieldsToText: true },
  { action: "cancelJob", codes: ["Period"], key: ".", ...ctrl },
  { action: "retryJob", codes: ["KeyR"], key: "r", ...ctrl, shift: true },
  { action: "keep", codes: ["KeyP"], key: "p", ctrl: false, alt: false, shift: false, yieldsToText: true },
  { action: "reject", codes: ["KeyX"], key: "x", ctrl: false, alt: false, shift: false, yieldsToText: true },
  {
    action: "clearCulling",
    codes: ["KeyU"],
    key: "u",
    ctrl: false,
    alt: false,
    shift: false,
    yieldsToText: true,
  },
];
