import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { PagedFeedFooter } from "./PagedFeedFooter";

describe("PagedFeedFooter", () => {
  it("shows a failed page with Try Again", () => {
    const loadMore = vi.fn();
    render(<PagedFeedFooter failure={{ title: "Can't load more" }} hasMore count={20} loadMore={loadMore} />);
    expect(screen.getByText("Can't load more")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Try Again" }));
    expect(loadMore).toHaveBeenCalledOnce();
  });

  it("loads the next page once shown, and again for each new count", () => {
    const loadMore = vi.fn();
    const { rerender } = render(<PagedFeedFooter hasMore count={20} loadMore={loadMore} />);
    expect(loadMore).toHaveBeenCalledTimes(1);
    rerender(<PagedFeedFooter hasMore count={20} loadMore={loadMore} />);
    expect(loadMore).toHaveBeenCalledTimes(1);
    rerender(<PagedFeedFooter hasMore count={40} loadMore={loadMore} />);
    expect(loadMore).toHaveBeenCalledTimes(2);
  });

  it("waits until it is scrolled into view", () => {
    let reveal: (() => void) | undefined;
    vi.stubGlobal(
      "IntersectionObserver",
      class {
        constructor(callback: (entries: { isIntersecting: boolean }[]) => void) {
          reveal = () => {
            callback([{ isIntersecting: true }]);
          };
        }
        observe() {}
        disconnect() {}
      },
    );
    const loadMore = vi.fn();
    render(<PagedFeedFooter hasMore count={20} loadMore={loadMore} />);
    expect(loadMore).not.toHaveBeenCalled();
    reveal?.();
    expect(loadMore).toHaveBeenCalledOnce();
  });
});
