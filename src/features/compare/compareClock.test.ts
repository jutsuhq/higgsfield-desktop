import { describe, expect, it } from "vitest";

import { compareDuration, dividerAfterKey, dividerAt, hasDrifted, sourceTime, timecode } from "./compareClock";

describe("compareClock", () => {
  it("stops at the shortest video", () => {
    expect(compareDuration([6, 4.5, 8])).toBe(4.5);
    expect(compareDuration([Number.NaN, 0, 3])).toBe(3);
    expect(compareDuration([])).toBe(0);
  });

  it("moves a video back once it drifts past a tenth of a second", () => {
    expect(hasDrifted(1.05, 1)).toBe(false);
    expect(hasDrifted(1.2, 1)).toBe(true);
  });

  it("plays the source from its range and never past its end", () => {
    expect(sourceTime(4, 10, 2)).toBe(6);
    expect(sourceTime(4, 10, 9)).toBe(10);
    expect(sourceTime(4, 10, -1)).toBe(4);
  });

  it("steps the divider by 5% within the player", () => {
    expect(dividerAfterKey(0.5, "ArrowLeft")).toBeCloseTo(0.45);
    expect(dividerAfterKey(0.98, "ArrowRight")).toBe(1);
    expect(dividerAfterKey(0.5, "Enter")).toBeNull();
    expect(dividerAt(50, 200)).toBe(0.25);
    expect(dividerAt(-10, 200)).toBe(0);
  });

  it("writes the timecode", () => {
    expect(timecode(4.25)).toBe("0:04.3");
    expect(timecode(65)).toBe("1:05.0");
    expect(timecode(Number.NaN)).toBe("0:00.0");
  });
});
