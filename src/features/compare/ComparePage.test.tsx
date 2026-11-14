import { fireEvent, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { BatchView_Serialize, CompareView_Serialize } from "../../bindings";
import { renderRoute } from "../../test/renderRoute";
import { noScore } from "../library/libraryTesting";

const file = (name: string) => ({ path: `/tmp/${name}.mp4`, url: `asset://${name}.mp4`, exists: true });

const compare: CompareView_Serialize = {
  tracks: [
    { id: "a", title: "rooftop · Restyle", labels: ["Original", "Restyle"], file: file("a") },
    { id: "b", title: "rooftop · Film Noir", labels: ["Film Noir", "480p"], file: file("b") },
  ],
  layouts: [
    { value: "sideBySide", title: "Side by Side" },
    { value: "divider", title: "Divider" },
  ],
  columns: 2,
};

const batch: BatchView_Serialize = {
  id: "a",
  title: "rooftop",
  subtitle: "Batch × 2",
  cells: [
    { id: "a", label: "Style 1", file: file("a"), isStarred: true, culling: "unreviewed", virality: noScore, canRenderFinal: true },
    {
      id: "b",
      label: "Style 2",
      file: file("b"),
      isStarred: false,
      culling: "unreviewed",
      virality: { ...noScore, badge: "72" },
      canRenderFinal: false,
    },
  ],
  columns: 2,
  unstarredIds: ["b"],
  score: { availability: { kind: "available" }, title: "Score Variants…", blocker: null, isRunning: false },
  canRenderFinals: true,
  hasStarredDrafts: true,
  finalsHelp: "Render the starred drafts as finals at one resolution",
  memberIds: ["a", "b"],
};

describe("ComparePage", () => {
  it("shows the results in the route's order and swaps their sides", async () => {
    const { calls } = await renderRoute("/library/compare/a,b", { library_compare: () => compare });
    expect(await screen.findByText("rooftop · Restyle")).toBeInTheDocument();
    expect(calls.find((call) => call.command === "library_compare")?.args).toEqual({ ids: ["a", "b"] });
    const titles = () => screen.getAllByText(/^rooftop · /).map((element) => element.textContent);
    expect(titles()).toEqual(["rooftop · Restyle", "rooftop · Film Noir"]);
    fireEvent.click(screen.getByRole("button", { name: "Swap Sides" }));
    expect(titles()).toEqual(["rooftop · Film Noir", "rooftop · Restyle"]);
    fireEvent.keyDown(window, { key: "ArrowLeft" });
    expect(titles()).toEqual(["rooftop · Restyle", "rooftop · Film Noir"]);
  });

  it("lays the second video over the first behind a divider", async () => {
    await renderRoute("/library/compare/a,b", { library_compare: () => compare });
    fireEvent.click(await screen.findByRole("button", { name: "Divider" }));
    const divider = screen.getByRole("slider", { name: "Compare divider" });
    expect(divider).toHaveAttribute("aria-valuenow", "50");
    fireEvent.keyDown(divider, { key: "ArrowRight" });
    expect(Number(divider.getAttribute("aria-valuenow"))).toBeGreaterThan(50);
  });

  it("says when the results can't be compared", async () => {
    await renderRoute("/library/compare/a", {
      library_compare: () => {
        throw new Error("Pick two or more results.");
      },
    });
    expect(await screen.findByText("These results can't be compared.")).toBeInTheDocument();
  });
});

describe("BatchPage", () => {
  it("shows each cell's label, score and Pick winner for a draft", async () => {
    await renderRoute("/library/batch/a", { library_batch: () => batch });
    expect(await screen.findAllByText("Style 1")).toHaveLength(2);
    expect(screen.getByText("72")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Pick winner/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Open/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Render Finals for Starred/ })).toBeEnabled();
  });

  it("asks for the price before scoring the batch", async () => {
    const { calls } = await renderRoute("/library/batch/a", {
      library_batch: () => batch,
      library_score_sheet: () => ({
        title: "Score 2 Variants",
        notice: null,
        priceText: "Free on your plan.",
        priceTag: null,
        needsConsent: false,
        consentText: "",
        blocker: null,
        buttonTitle: "Score",
      }),
      library_score: () => ({ kind: "started" }),
    });
    fireEvent.click(await screen.findByRole("button", { name: /Score Variants/ }));
    expect(await screen.findByText("Free on your plan.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Score" }));
    await waitFor(() => {
      expect(calls.find((call) => call.command === "library_score")?.args).toEqual({ ids: ["a", "b"], approved: false });
    });
  });
});
