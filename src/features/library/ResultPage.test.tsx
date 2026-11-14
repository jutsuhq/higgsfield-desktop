import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { ResultDetail_Serialize } from "../../bindings";
import { renderRoute } from "../../test/renderRoute";
import { defaultFilter, useLibraryView } from "./libraryData";
import { emptyMenu, resultDetail } from "./libraryTesting";

function details(byId: Record<string, ResultDetail_Serialize>) {
  return {
    library_detail: (args: Record<string, unknown>) => {
      const found = byId[String(args.id)];
      if (!found) throw new Error("This result is no longer in the library.");
      return found;
    },
    library_cull: (args: Record<string, unknown>) => ({ action: args.action, cursor: null }),
    library_undo_culling: () => "final",
  };
}

const final = resultDetail("final", {
  title: "rooftop · Final",
  isDraft: false,
  chain: [
    { id: "draft", title: "draft 480p", isDeleted: false, isCurrent: false },
    { id: "final", title: "final 1080p", isDeleted: false, isCurrent: true },
  ],
  madeFrom: [
    {
      reference: { materialID: "img", versionID: "v1" },
      isVideo: false,
      roleTitle: "Start frame",
      prompt: "a rooftop",
      previewUrl: null,
      availability: "listed",
      note: null,
    },
  ],
});
const draft = resultDetail("draft", { title: "rooftop · Draft" });

afterEach(() => {
  useLibraryView.setState({ origin: "local", kind: "all", filter: defaultFilter, expandedGroups: [], order: [] });
});

describe("Result page", () => {
  it("shows the player over the source, the details, the chain and what it was made from", async () => {
    await renderRoute("/library/result/final", details({ final, draft }));
    expect(await screen.findByRole("heading", { level: 1, name: "rooftop · Final" })).toBeInTheDocument();
    expect(screen.getByText("Before")).toBeInTheDocument();
    expect(screen.getByText("After")).toBeInTheDocument();
    expect([...document.querySelectorAll("video")].map((video) => video.getAttribute("src"))).toEqual(
      expect.arrayContaining(["http://127.0.0.1:1/clip.mov", "http://127.0.0.1:1/final.mp4"]),
    );
    expect(screen.getByRole("slider", { name: "Compare divider" })).toBeInTheDocument();
    expect(screen.getByText("A clay rooftop at dusk")).toBeInTheDocument();
    expect(screen.getByText("$1.91 · API key")).toBeInTheDocument();
    expect(screen.getByText("clip.mov")).toBeInTheDocument();
    expect(screen.getByText("00:04.0 – 00:10.0")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "final 1080p" })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Start frame/ })).toBeInTheDocument();
  });

  it("opens the draft from the chain", async () => {
    await renderRoute("/library/result/final", details({ final, draft }));
    fireEvent.click(await screen.findByRole("button", { name: "draft 480p" }));
    expect(await screen.findByRole("heading", { level: 1, name: "rooftop · Draft" })).toBeInTheDocument();
  });

  it("culls, stars and undoes from the keyboard and its buttons", async () => {
    const { calls } = await renderRoute("/library/result/final", details({ final, draft }));
    await screen.findByRole("heading", { level: 1, name: "rooftop · Final" });
    fireEvent.keyDown(window, { key: "x", code: "KeyX" });
    fireEvent.keyDown(window, { key: "z", code: "KeyZ", ctrlKey: true });
    fireEvent.click(screen.getByRole("button", { name: "Star" }));
    await waitFor(() => {
      expect(calls.find((call) => call.command === "library_cull")?.args).toMatchObject({ action: "reject", ids: ["final"] });
      expect(calls.some((call) => call.command === "library_undo_culling")).toBe(true);
      expect(calls.find((call) => call.command === "library_set_starred")?.args).toEqual({
        ids: ["final"],
        isStarred: true,
      });
    });
  });

  it("steps on to the next result after a verdict, as LibraryView.cullShown does", async () => {
    useLibraryView.setState({ order: ["final", "draft"] });
    const { router } = await renderRoute("/library/result/final", {
      ...details({ final, draft }),
      library_cull: (args: Record<string, unknown>) => ({ action: args.action, cursor: "draft" }),
    });
    await screen.findByRole("heading", { level: 1, name: "rooftop · Final" });
    fireEvent.keyDown(window, { key: "p", code: "KeyP" });
    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/library/result/draft");
    });
  });

  it("steps through the grid's results with the arrows and goes back with Esc", async () => {
    useLibraryView.setState({ order: ["draft", "final"] });
    const { router } = await renderRoute("/library/result/final", details({ final, draft }));
    await screen.findByText("2 of 2");
    fireEvent.keyDown(window, { key: "ArrowLeft" });
    expect(await screen.findByRole("heading", { level: 1, name: "rooftop · Draft" })).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/library");
    });
  });

  it("lists More in the Swift order, with Compare with… as a submenu", async () => {
    const editable = resultDetail("final", { menu: { ...emptyMenu, canCompare: true, canEditVideo: true } });
    const { calls, router } = await renderRoute("/library/result/final", {
      ...details({ final: editable }),
      library_compare_candidates: () => [{ id: "other", title: "rooftop · Retake" }],
    });
    fireEvent.click(await screen.findByRole("button", { name: "More" }));
    const menu = await screen.findByRole("menu");
    await within(menu).findByRole("menuitem", { name: /Compare with…/ });
    expect(within(menu).getAllByRole("menuitem").map((item) => item.textContent)).toEqual([
      "Duplicate",
      "Edit…",
      "Extend…",
      "Share…",
      "Copy Prompt",
      "Copy Request ID",
      "Compare with…›",
      "Save as Template…",
      "Delete from Library…",
    ]);
    fireEvent.click(within(menu).getByRole("menuitem", { name: /Compare with…/ }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "rooftop · Retake" }));
    await waitFor(() => {
      expect(decodeURIComponent(router.state.location.pathname)).toBe("/library/compare/final,other");
    });
    expect(calls.some((call) => call.command === "library_compare_candidates")).toBe(true);
  });

  it("renders a draft's final at the picked resolution and asks about the cap first", async () => {
    const toRender = resultDetail("draft", { menu: { ...emptyMenu, followUp: "renderFinal", canRerun: true, canJoin: true } });
    const section = (selected: string) => ({
      options: [
        { value: "720p", title: "720p" },
        { value: "1080p", title: "1080p" },
      ],
      selected,
      detail: null,
      window: null,
      buttonTitle: "Render final",
      help: null,
      price: "$3.40",
      isQueued: false,
      isConnected: true,
    });
    let approved = false;
    const { calls } = await renderRoute("/library/result/draft", {
      ...details({ draft: toRender }),
      library_final_section: (args: Record<string, unknown>) => section((args.resolution as string | null) ?? "1080p"),
      library_render_final: (args: Record<string, unknown>) => {
        approved = args.approved === true;
        return approved
          ? { kind: "queued", notice: "Final queued" }
          : {
              kind: "confirm",
              confirmation: { title: "Over today's spend cap", message: "It adds $3.40.", confirmTitle: "Render Anyway" },
            };
      },
    });
    const box = await screen.findByRole("region", { name: "Render final" });
    fireEvent.click(within(box).getByRole("button", { name: "720p" }));
    await waitFor(() => {
      expect(calls.some((call) => call.command === "library_final_section" && call.args.resolution === "720p")).toBe(true);
    });
    fireEvent.click(await within(box).findByRole("button", { name: /^Render final/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Render Anyway" }));
    await waitFor(() => {
      const renders = calls.filter((call) => call.command === "library_render_final").map((call) => call.args);
      expect(renders).toEqual([
        { id: "draft", resolution: "720p", approved: false },
        { id: "draft", resolution: "720p", approved: true },
      ]);
    });
    expect(screen.getByRole("button", { name: "Edit & Re-run" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Join with Original…" }));
    await waitFor(() => {
      expect(calls.find((call) => call.command === "library_join_with_original")?.args).toEqual({ id: "draft" });
    });
  });

  it("says when the result is gone", async () => {
    await renderRoute("/library/result/missing", details({}));
    expect(await screen.findByText("This result is no longer in the library.")).toBeInTheDocument();
  });
});

describe("Duplicate and Save as Template…", () => {
  it("queues the request again, asking first over today's cap", async () => {
    let asked = false;
    const { calls } = await renderRoute("/library/result/final", {
      ...details({ final }),
      queue_duplicate: (args: Record<string, unknown>) => {
        if (args.overCapApproved === true) return { kind: "queued", notice: "Queued · ~$0.68" };
        asked = true;
        return {
          kind: "confirm",
          confirmation: { title: "Over today's cap", message: "Queue it anyway?", confirmTitle: "Queue Anyway" },
        };
      },
    });
    fireEvent.click(await screen.findByRole("button", { name: "More" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Duplicate" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Over today's cap" });
    expect(asked).toBe(true);
    fireEvent.click(within(dialog).getByRole("button", { name: "Queue Anyway" }));
    await waitFor(() => {
      expect(calls.filter((call) => call.command === "queue_duplicate").map((call) => call.args)).toEqual([
        { id: "final", overCapApproved: false },
        { id: "final", overCapApproved: true },
      ]);
    });
  });

  it("saves the result's inputs under the name Rust suggests", async () => {
    const { calls } = await renderRoute("/library/result/final", {
      ...details({ final }),
      library_template_name: () => "Motion · A clay rooftop",
      library_save_template: (args: Record<string, unknown>) => ({ kind: "saved", name: args.name }),
    });
    fireEvent.click(await screen.findByRole("button", { name: "More" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Save as Template…" }));
    const field = await screen.findByRole("textbox", { name: "Name" });
    expect(field).toHaveValue("Motion · A clay rooftop");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => {
      expect(calls.find((call) => call.command === "library_save_template")?.args).toEqual({
        id: "final",
        name: "Motion · A clay rooftop",
      });
    });
  });
});
