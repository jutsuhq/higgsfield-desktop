import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { QueueConfirmation, QueueRow_Serialize, QueueSnapshot_Serialize } from "../../bindings";
import { emptyImages, emptyQueue } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";

const requests = [
  {
    id: "run-1",
    title: "Soul 2 · 4 images",
    status: "working" as const,
    detail: "Making the images…",
    canCancel: true,
    subtitle: null,
    review: null,
  },
  {
    id: "run-2",
    title: "Soul 2 · 1 image",
    status: "failed" as const,
    detail: "The model is busy",
    canCancel: false,
    subtitle: null,
    review: null,
  },
];

function member(id: string): QueueRow_Serialize {
  return {
    model: {
      id,
      title: id,
      isDraft: true,
      meta: "Kling 3.0 · 5s",
      cost: null,
      priceSource: null,
      isCharged: false,
      statusLabel: "Waiting",
      statusRole: "muted",
      statusHint: null,
      isWorking: false,
      progress: null,
      progressText: null,
      note: null,
      actions: [],
      stage: null,
      elapsed: null,
      generating: null,
      timeEstimate: null,
      prompt: "",
      origin: null,
      originDetail: null,
    },
    commands: { canCancel: true, retry: null, retryTitle: "Retry" },
    supportIds: null,
    preview: {
      hasSource: false,
      source: null,
      references: [],
      moreReferences: 0,
      result: { kind: "rendering" },
      aspect: null,
    },
    priceConfirmation: null,
    isFresh: false,
  } as unknown as QueueRow_Serialize;
}

const withBatch: QueueSnapshot_Serialize = {
  ...emptyQueue,
  activeCount: 2,
  active: [
    {
      kind: "batch",
      batch: {
        id: "batch-1",
        title: "3 styles",
        progress: null,
        progressText: "0 of 2 done",
        totalCostText: null,
        isActive: true,
        canCancelAll: true,
        rows: [member("job-1"), member("job-2")],
      },
    },
  ],
};

describe("Queue › Images and Cancel All", () => {
  it("lists the image requests instead of the empty state, cancels one and shows them in Create", async () => {
    const { calls, router } = await renderRoute("/queue", {
      images_snapshot: () => ({ ...emptyImages, requests }),
    });
    expect(await screen.findByText("Soul 2 · 4 images")).toBeTruthy();
    expect(screen.queryByText("Nothing running")).toBeNull();
    expect(screen.getByText("The model is busy")).toBeTruthy();
    const list = screen.getByRole("list", { name: "Image requests" });
    expect(within(list).getAllByRole("button", { name: "Cancel" })).toHaveLength(1);
    fireEvent.click(within(list).getByRole("button", { name: "Cancel" }));
    fireEvent.click(screen.getByRole("button", { name: "Show in Create" }));
    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/create");
    });
    expect(calls.find((call) => call.command === "images_cancel")?.args.id).toBe("run-1");
    expect(calls.find((call) => call.command === "images_set_side")?.args.side).toBe("image");
  });

  it("reviews an agent's held request: Not Now keeps it, Send sends it, Cancel Request cancels it", async () => {
    const held = {
      id: "run-3",
      title: "Mara · Soul 2 · 4 images",
      status: "waiting" as const,
      detail: "Waiting for you: over today's agent budget",
      canCancel: true,
      subtitle: "Agent · Claude · Allowed up to $0.40",
      review: "Claude\nMara · Soul 2 · 4 images\n$0.60",
    };
    const { calls } = await renderRoute("/queue", {
      images_snapshot: () => ({ ...emptyImages, requests: [held] }),
    });
    expect(await screen.findByText("Agent · Claude · Allowed up to $0.40")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Review…" }));
    let dialog = await screen.findByRole("alertdialog", { name: "Send this agent's request?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Not Now" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Review…" }));
    dialog = await screen.findByRole("alertdialog", { name: "Send this agent's request?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Send" }));
    fireEvent.click(screen.getByRole("button", { name: "Review…" }));
    dialog = await screen.findByRole("alertdialog", { name: "Send this agent's request?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel Request" }));
    await waitFor(() => {
      expect(calls.find((call) => call.command === "agents_send_held_image")?.args.id).toBe("run-3");
    });
    expect(calls.find((call) => call.command === "images_cancel")?.args.id).toBe("run-3");
  });

  it("asks before Cancel All once a job is with Higgsfield, and keeps them on Keep Them", async () => {
    const question: QueueConfirmation = {
      title: "Cancel 2 jobs?",
      message:
        "1 of them is already with Higgsfield, which is asked to drop them. A job it already started can't be canceled and is charged.",
      confirmTitle: "Cancel 2 Jobs",
    };
    const { calls } = await renderRoute("/queue", {
      queue_snapshot: () => withBatch,
      queue_cancel_all: (args) => (args.approved ? null : question),
    });
    fireEvent.click(await screen.findByRole("button", { name: "Cancel All" }));
    let dialog = await screen.findByRole("alertdialog", { name: "Cancel 2 jobs?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Keep Them" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Cancel All" }));
    dialog = await screen.findByRole("alertdialog", { name: "Cancel 2 jobs?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel 2 Jobs" }));
    await waitFor(() => {
      expect(calls.filter((call) => call.command === "queue_cancel_all").map((call) => call.args)).toEqual([
        { id: "batch-1", approved: false },
        { id: "batch-1", approved: false },
        { id: "batch-1", approved: true },
      ]);
    });
  });

  it("has no Cancel All when nothing in the batch can be canceled", async () => {
    const done = structuredClone(withBatch);
    const entry = done.active[0];
    if (entry?.kind === "batch") entry.batch.canCancelAll = false;
    await renderRoute("/queue", { queue_snapshot: () => done });
    expect(await screen.findByText("3 styles")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Cancel All" })).toBeNull();
  });
});
