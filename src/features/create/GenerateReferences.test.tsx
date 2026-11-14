import { QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { ImageSnapshot_Serialize, ReferencePicksView_Serialize } from "../../bindings";
import { createQueryClient } from "../../lib/queryClient";
import { emptyImages, mockCommands, type CommandHandlers } from "../../lib/testing/ipcMock";
import { GenerateReferenceSheet } from "./GenerateReferences";
import { renderRoute } from "../../test/renderRoute";

const withResults: ImageSnapshot_Serialize = {
  ...emptyImages,
  side: "image",
  prompt: "A red fox",
  generate: { isGenerating: false, enabled: true, price: "$0.0032", hint: "$0.0032 · Price from Higgsfield" },
  notice: null,
  images: [
    {
      id: "img-2",
      path: null,
      url: "https://example.com/2.png",
      prompt: "A blue fox",
      model: "Soul 2",
      isPicked: false,
    },
    {
      id: "img-1",
      path: null,
      url: "https://example.com/1.png",
      prompt: "A red fox",
      model: "Soul 2",
      isPicked: false,
    },
  ],
};

const none: ReferencePicksView_Serialize = {
  offered: true,
  picked: [],
  added: [],
  canPick: true,
  detail: "Pick up to 2",
};

const both: ReferencePicksView_Serialize = {
  ...none,
  picked: ["img-1", "img-2"],
  canPick: false,
  detail: "2 of 2 picked",
};

function renderSheet(handlers: CommandHandlers, onClose: () => void = () => undefined) {
  const calls = mockCommands(handlers);
  const result = render(
    <QueryClientProvider client={createQueryClient()}>
      <GenerateReferenceSheet onClose={onClose} />
    </QueryClientProvider>,
  );
  return { calls, ...result };
}

describe("References › Generate…", () => {
  it("is offered on References with an image connection", async () => {
    await renderRoute("/create", { images_reference_picks: () => none });
    fireEvent.click(await screen.findByRole("button", { name: "Generate…" }));
    expect(await screen.findByText("Generate reference images", { selector: "h2" })).toBeTruthy();
  });

  it("picks results through Rust and numbers them in the order picked", async () => {
    const { calls } = renderSheet({
      images_snapshot: () => withResults,
      images_reference_picks: () => none,
      images_toggle_reference_pick: () => both,
    });
    fireEvent.click((await screen.findAllByRole("button", { name: "Generated image" }))[0] as HTMLElement);
    expect(await screen.findByText("2 of 2 picked")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Generated image, picked 1" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Generated image, picked 2" })).toBeTruthy();
    expect(
      calls.filter((call) => call.command === "images_toggle_reference_pick").map((call) => call.args.id),
    ).toEqual(["img-2"]);
    expect(screen.getByRole("button", { name: /Add 2 to References/ })).toBeTruthy();
  });

  it("adds the picks in the order picked, then closes", async () => {
    let closed = false;
    const { calls } = renderSheet(
      {
        images_snapshot: () => withResults,
        images_reference_picks: () => both,
        create_add_generated: () => null,
      },
      () => {
        closed = true;
      },
    );
    fireEvent.click(await screen.findByRole("button", { name: /Add 2 to References/ }));
    await waitFor(() => {
      expect(closed).toBe(true);
    });
    expect(
      calls.filter((call) => call.command === "create_add_generated").map((call) => call.args.id),
    ).toEqual(["img-1", "img-2"]);
  });

  it("stops at a pick that wasn't added and says why", async () => {
    let closed = false;
    const { calls } = renderSheet(
      {
        images_snapshot: () => withResults,
        images_reference_picks: () => both,
        create_add_generated: () => "Can't download the image",
      },
      () => {
        closed = true;
      },
    );
    fireEvent.click(await screen.findByRole("button", { name: /Add 2 to References/ }));
    expect(await screen.findByText("Can't download the image")).toBeTruthy();
    expect(closed).toBe(false);
    expect(calls.filter((call) => call.command === "create_add_generated")).toHaveLength(1);
  });

  it("asks before closing with picks and drops them when it closes", async () => {
    let closed = false;
    const { calls, unmount } = renderSheet(
      { images_snapshot: () => withResults, images_reference_picks: () => both },
      () => {
        closed = true;
      },
    );
    await screen.findByText("2 of 2 picked");
    fireEvent.click(screen.getAllByRole("button", { name: "Close" })[0] as HTMLElement);
    const alert = await screen.findByRole("alertdialog", { name: "Close without adding the picked images?" });
    fireEvent.click(within(alert).getByRole("button", { name: "Keep Picking" }));
    expect(closed).toBe(false);
    fireEvent.click(screen.getAllByRole("button", { name: "Close" })[0] as HTMLElement);
    fireEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Close" }));
    expect(closed).toBe(true);
    unmount();
    expect(calls.some((call) => call.command === "images_clear_reference_picks")).toBe(true);
  });

  it("opens a result large on a double click, steps through the results and picks with Space", async () => {
    const { calls } = renderSheet({
      images_snapshot: () => withResults,
      images_reference_picks: () => none,
      images_toggle_reference_pick: () => none,
    });
    fireEvent.doubleClick(
      (await screen.findAllByRole("button", { name: "Generated image" }))[1] as HTMLElement,
    );
    const preview = await screen.findByRole("dialog", { name: "Generated image" });
    expect(within(preview).getByText("Image 2 of 2")).toBeTruthy();
    expect(within(preview).getByText("Soul 2")).toBeTruthy();
    expect(within(preview).getByRole("img", { name: "A red fox" })).toBeTruthy();
    expect(within(preview).getByRole("button", { name: "Next" })).toBeDisabled();
    expect(within(preview).getByRole("button", { name: "Pick" })).toBeEnabled();
    fireEvent.keyDown(window, { key: " " });
    await waitFor(() => {
      expect(
        calls.filter((call) => call.command === "images_toggle_reference_pick").map((call) => call.args.id),
      ).toEqual(["img-1"]);
    });
    fireEvent.click(within(preview).getByRole("button", { name: "Previous" }));
    expect(await screen.findByText("Image 1 of 2")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Back to Results" }));
    expect(screen.queryByRole("dialog", { name: "Generated image" })).toBeNull();
  });
});
