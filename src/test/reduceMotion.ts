import { vi } from "vitest";

/** Turns Reduce Motion on (`prefers-reduced-motion: reduce`) for the rest of the test. */
export function reduceMotion(): void {
  vi.stubGlobal(
    "matchMedia",
    (query: string): MediaQueryList =>
      ({
        matches: query.includes("prefers-reduced-motion"),
        media: query,
        onchange: null,
        addEventListener: () => undefined,
        removeEventListener: () => undefined,
        dispatchEvent: () => false,
      }) as unknown as MediaQueryList,
  );
}
