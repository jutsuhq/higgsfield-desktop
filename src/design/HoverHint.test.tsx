import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { HoverHint, hintDelayMs } from "./HoverHint";

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

const hint = () => screen.queryByRole("tooltip", { hidden: true });

function rest(element: HTMLElement, ms: number) {
  fireEvent.pointerEnter(element);
  act(() => {
    vi.advanceTimersByTime(ms);
  });
}

describe("HoverHint", () => {
  it("shows after the pointer rests 400 ms and hides when it leaves", () => {
    render(
      <HoverHint text="Compare two results">
        <button>Compare</button>
      </HoverHint>,
    );
    const anchor = screen.getByRole("button").parentElement as HTMLElement;
    rest(anchor, hintDelayMs - 1);
    expect(hint()).not.toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(hint()).toHaveTextContent("Compare two results");
    fireEvent.pointerLeave(anchor);
    expect(hint()).not.toBeInTheDocument();
  });

  it("hides when the page scrolls, and only one shows at a time", () => {
    render(
      <>
        <HoverHint text="First">
          <button>1</button>
        </HoverHint>
        <HoverHint text="Second">
          <button>2</button>
        </HoverHint>
      </>,
    );
    const [first, second] = screen
      .getAllByRole("button")
      .map((button) => button.parentElement as HTMLElement);
    rest(first as HTMLElement, hintDelayMs);
    rest(second as HTMLElement, hintDelayMs);
    expect(screen.getAllByRole("tooltip", { hidden: true })).toHaveLength(1);
    expect(hint()).toHaveTextContent("Second");
    fireEvent.scroll(window);
    expect(hint()).not.toBeInTheDocument();
  });
});
