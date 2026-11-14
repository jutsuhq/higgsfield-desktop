import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { RemoteThumbnail } from "./RemoteThumbnail";

const image = () => document.querySelector("img");

describe("RemoteThumbnail", () => {
  it("loads only an https link", () => {
    const { rerender } = render(<RemoteThumbnail url="http://example.com/a.webp" />);
    expect(image()).toBeNull();
    rerender(<RemoteThumbnail url={null} />);
    expect(image()).toBeNull();
    rerender(<RemoteThumbnail url="https://example.com/a.webp" />);
    expect(image()).not.toBeNull();
  });

  it("shows nothing when the picture fails to load", () => {
    render(<RemoteThumbnail url="https://example.com/broken.webp" />);
    fireEvent.error(image() as HTMLImageElement);
    expect(image()).toBeNull();
    expect(screen.queryByRole("img")).toBeNull();
  });
});
