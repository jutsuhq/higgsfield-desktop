import { describe, expect, it } from "vitest";

import {
  chosen,
  cullingHint,
  cullingTargets,
  emptySelection,
  extended,
  movedCursor,
  selectedAll,
  toggled,
} from "./selection";

const order = ["a", "b", "c", "d", "e"];

describe("LibrarySelection", () => {
  it("toggles a card and remembers it as the anchor", () => {
    const one = toggled(emptySelection, "b");
    expect([...one.ids]).toEqual(["b"]);
    expect(one.anchor).toBe("b");
    const none = toggled(one, "b");
    expect(none.ids.size).toBe(0);
    expect(none.anchor).toBeNull();
  });

  it("extends from the anchor to the clicked card in grid order, either way", () => {
    const down = extended(toggled(emptySelection, "b"), "d", order);
    expect(chosen(down, order)).toEqual(["b", "c", "d"]);
    const up = extended(toggled(emptySelection, "d"), "a", order);
    expect(chosen(up, order)).toEqual(["a", "b", "c", "d"]);
  });

  it("extends without an anchor by toggling", () => {
    expect(chosen(extended(emptySelection, "c", order), order)).toEqual(["c"]);
  });

  it("selects every card shown", () => {
    const all = selectedAll(order);
    expect(chosen(all, order)).toEqual(order);
    expect(all.anchor).toBe("a");
  });
});

describe("CullingTargets", () => {
  it("acts on the visible selected cards, else on the current one", () => {
    const picked = toggled(toggled(emptySelection, "c"), "a");
    expect(cullingTargets(order, picked, "e")).toEqual(["a", "c"]);
    expect(cullingTargets(order, emptySelection, "e")).toEqual(["e"]);
  });

  it("never acts on a hidden card", () => {
    const hidden = toggled(emptySelection, "z");
    expect(cullingTargets(order, hidden, null)).toEqual([]);
    expect(cullingTargets(order, emptySelection, "z")).toEqual([]);
  });

  it("names the action with its count", () => {
    expect(cullingHint("keep", 1)).toBe("Keep");
    expect(cullingHint("reject", 3)).toBe("Reject 3 results");
    expect(cullingHint("clear", 1)).toBe("Clear verdict");
    expect(cullingHint("clear", 3)).toBe("Clear 3 verdicts");
  });
});

describe("LibraryCursor", () => {
  it("starts on the first card", () => {
    expect(movedCursor(null, "right", order, 2)).toBe("a");
    expect(movedCursor("gone", "down", order, 2)).toBe("a");
  });

  it("steps one card sideways and a row of columns up and down, stopping at the ends", () => {
    expect(movedCursor("b", "right", order, 2)).toBe("c");
    expect(movedCursor("b", "left", order, 2)).toBe("a");
    expect(movedCursor("a", "left", order, 2)).toBe("a");
    expect(movedCursor("b", "down", order, 2)).toBe("d");
    expect(movedCursor("d", "down", order, 2)).toBe("e");
    expect(movedCursor("b", "up", order, 2)).toBe("a");
  });

  it("is empty without cards", () => {
    expect(movedCursor(null, "down", [], 3)).toBeNull();
  });
});
