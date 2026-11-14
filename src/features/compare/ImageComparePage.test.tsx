import { fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { ImageCompareSides_Serialize, ImageCompareView_Serialize } from "../../bindings";
import { renderRoute } from "../../test/renderRoute";
import { defaultFilter, useLibraryView } from "../library/libraryData";

const image = (id: string, title: string) => ({ id, title, url: `http://127.0.0.1:1/${id}.png` });

const pair: ImageCompareView_Serialize = {
  images: [image("c", "A knight"), image("d", "lake.png")],
  sides: { left: 0, right: 1, position: null },
};

const variants: ImageCompareView_Serialize = {
  images: [image("v1", "Variant one"), image("v2", "Variant two"), image("v3", "Variant three")],
  sides: { left: 0, right: 1, position: "Variant 2 of 3" },
};

/** Rust's sides for three variants or two images, as `ImageCompareSides.changed` answers. */
function sidesAfter(args: Record<string, unknown>): ImageCompareSides_Serialize {
  const sides = args.sides as ImageCompareSides_Serialize;
  const count = args.count as number;
  if (args.change === "swap") {
    return {
      left: sides.right,
      right: sides.left,
      position: sides.position === null ? null : `Variant ${String(sides.left + 1)} of ${String(count)}`,
    };
  }
  if (sides.position === null) return sides;
  let right = sides.right;
  do {
    right = (right + (args.change === "next" ? 1 : count - 1)) % count;
  } while (right === sides.left);
  return { left: sides.left, right, position: `Variant ${String(right + 1)} of ${String(count)}` };
}

const titles = () => screen.getAllByRole("img").map((pane) => pane.getAttribute("aria-label"));

afterEach(() => {
  useLibraryView.setState({ origin: "local", kind: "all", filter: defaultFilter, expandedGroups: [], order: [] });
});

describe("ImageComparePage", () => {
  it("shows the two picked images and swaps their sides with the button and the arrows", async () => {
    const { calls } = await renderRoute("/library/compare-images/c,d", {
      library_image_compare: () => pair,
      library_image_compare_sides: sidesAfter,
    });
    expect(await screen.findByText("A knight")).toBeInTheDocument();
    expect(calls.find((call) => call.command === "library_image_compare")?.args).toEqual({ ids: ["c", "d"] });
    expect(titles()).toEqual(["A knight", "lake.png"]);
    expect(screen.queryByRole("button", { name: "Next Variant" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Swap Sides" }));
    await waitFor(() => {
      expect(titles()).toEqual(["lake.png", "A knight"]);
    });
    fireEvent.keyDown(window, { key: "ArrowRight" });
    await waitFor(() => {
      expect(titles()).toEqual(["A knight", "lake.png"]);
    });
    // ↑ and ↓ page only a variant group.
    fireEvent.keyDown(window, { key: "ArrowDown" });
    expect(calls.filter((call) => call.command === "library_image_compare_sides").map((call) => call.args.change)).toEqual([
      "swap",
      "swap",
    ]);
  });

  it("fits, shows actual size and zooms both panes from one state", async () => {
    await renderRoute("/library/compare-images/c,d", { library_image_compare: () => pair });
    const fit = await screen.findByRole("button", { name: "Fit" });
    const actual = screen.getByRole("button", { name: "Actual Size" });
    const zoom = screen.getByRole("slider", { name: "Zoom" });
    expect(fit).toBeDisabled();
    expect(actual).toBeEnabled();
    fireEvent.click(actual);
    expect(actual).toBeDisabled();
    expect(fit).toBeEnabled();
    fireEvent.change(zoom, { target: { value: "20" } });
    expect(zoom).toHaveValue("8");
    expect(actual).toBeEnabled();
    fireEvent.click(fit);
    expect(fit).toBeDisabled();
    expect(zoom).toHaveValue("1");
  });

  it("says when the images can't be compared", async () => {
    await renderRoute("/library/compare-images/c", {
      library_image_compare: () => {
        throw new Error("Compare needs two images.");
      },
    });
    expect(await screen.findByText("These images can't be compared.")).toBeInTheDocument();
  });
});

describe("ImageVariantsPage", () => {
  it("asks for the group as the grid shows it and steps the right side with the pager and the arrows", async () => {
    useLibraryView.setState({ kind: "image", expandedGroups: ["images-1"] });
    const { calls } = await renderRoute("/library/compare-variants/images-1", {
      library_image_variants: () => variants,
      library_image_compare_sides: sidesAfter,
    });
    expect(await screen.findByText("Variant 2 of 3")).toBeInTheDocument();
    expect(calls.find((call) => call.command === "library_image_variants")?.args).toEqual({
      request: { origin: "local", kind: "image", filter: defaultFilter, expandedGroups: ["images-1"] },
      group: "images-1",
    });
    expect(titles()).toEqual(["Variant one", "Variant two"]);
    fireEvent.click(screen.getByRole("button", { name: "Next Variant" }));
    expect(await screen.findByText("Variant 3 of 3")).toBeInTheDocument();
    expect(titles()).toEqual(["Variant one", "Variant three"]);
    fireEvent.keyDown(window, { key: "ArrowDown" });
    expect(await screen.findByText("Variant 2 of 3")).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "ArrowUp" });
    expect(await screen.findByText("Variant 3 of 3")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Previous Variant" }));
    expect(await screen.findByText("Variant 2 of 3")).toBeInTheDocument();
  });
});
