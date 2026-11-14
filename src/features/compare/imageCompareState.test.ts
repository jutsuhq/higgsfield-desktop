import { describe, expect, it } from "vitest";

import { actualSizeState, displayScale, fitScale, fitState } from "./imageCompareState";

/** `ImageCompareTests` in `LibraryMediaTests.swift`. */
describe("ImageCompareState", () => {
  it("shows actual size one to one whatever the pane holds", () => {
    // A 4000px image in a 400pt pane fits at 0.1; Actual Size must show 4000, not 400.
    const fit = fitScale(4000, 3000, 400, 400);
    expect(fit).toBeCloseTo(0.1, 4);
    expect(displayScale(actualSizeState, fit)).toBe(1);
    expect(4000 * displayScale(actualSizeState, fit)).toBe(4000);
    expect(4000 * displayScale(fitState, fit)).toBeCloseTo(400, 6);
  });

  it("never fits past actual size", () => {
    expect(fitScale(100, 100, 400, 400)).toBe(1);
    expect(fitScale(0, 100, 400, 400)).toBe(1);
  });
});
