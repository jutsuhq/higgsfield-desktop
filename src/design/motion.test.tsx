import { renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { reduceMotion } from "../test/reduceMotion";
import { prefersReducedMotion, useReducedMotion } from "./motion";

describe("useReducedMotion", () => {
  it("follows prefers-reduced-motion", () => {
    reduceMotion();
    expect(prefersReducedMotion()).toBe(true);
    expect(renderHook(() => useReducedMotion()).result.current).toBe(true);
  });
});
