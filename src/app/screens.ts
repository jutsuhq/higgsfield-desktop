import { Film, Layers, Settings, Sparkles, type LucideIcon } from "lucide-react";

import type { ShortcutAction } from "../bindings";

export type ScreenId = "create" | "queue" | "library" | "settings";

export interface Screen {
  id: ScreenId;
  title: string;
  path: `/${ScreenId}`;
  icon: LucideIcon;
  /** The page's shortcut in Settings › Shortcuts; Settings has the fixed Ctrl+, instead. */
  shortcutAction: ShortcutAction | null;
}

/** In sidebar order; the pages are listed at the top, Settings sits at the bottom. */
export const screens: readonly Screen[] = [
  { id: "create", title: "Create", path: "/create", icon: Sparkles, shortcutAction: "create" },
  { id: "queue", title: "Queue", path: "/queue", icon: Layers, shortcutAction: "queue" },
  { id: "library", title: "Library", path: "/library", icon: Film, shortcutAction: "library" },
  { id: "settings", title: "Settings", path: "/settings", icon: Settings, shortcutAction: null },
];

export const pages = screens.filter((screen) => screen.id !== "settings");

export function screenFor(pathname: string): Screen | undefined {
  return screens.find((screen) => pathname === screen.path || pathname.startsWith(`${screen.path}/`));
}
