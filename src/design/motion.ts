import { useSyncExternalStore } from "react";

/**
 * Motion.swift: Reduce Motion turns animations off. CSS transitions and keyframes are switched off in
 * `motion.css`; this hook is for components that decide in code whether to run an animation at all, as the Swift
 * views do with `isActive: … && !reduceMotion`.
 */
const reduceMotionQuery = "(prefers-reduced-motion: reduce)";

function mediaQuery(): MediaQueryList | undefined {
  return typeof window.matchMedia === "function" ? window.matchMedia(reduceMotionQuery) : undefined;
}

function subscribe(onChange: () => void): () => void {
  const query = mediaQuery();
  query?.addEventListener("change", onChange);
  return () => query?.removeEventListener("change", onChange);
}

export function prefersReducedMotion(): boolean {
  return mediaQuery()?.matches ?? false;
}

/** `@Environment(\.accessibilityReduceMotion)`. */
export function useReducedMotion(): boolean {
  return useSyncExternalStore(subscribe, prefersReducedMotion, () => false);
}
