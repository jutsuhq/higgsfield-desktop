import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { QueueRow_Serialize, QueueSnapshot_Serialize, RowAction } from "../../bindings";
import { useToastStore } from "../../app/toastStore";
import { emitEvent, emptyQueue } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";

function row(id: string, title: string, actions: RowAction[], overrides: Partial<QueueRow_Serialize> = {}) {
  const value: QueueRow_Serialize = {
    model: {
      id,
      title,
      isDraft: false,
      meta: "Seedance 2.5 · 5s · $0.68",
      cost: "$0.68",
      priceSource: "Estimate",
      isCharged: false,
      statusLabel: "Generating",
      statusRole: "accent",
      statusHint: null,
      isWorking: true,
      progress: null,
      progressText: null,
      note: null,
      actions,
      stage: "generate",
      elapsed: "1:05",
      generating: null,
      timeEstimate: null,
      prompt: "A fox in the snow",
      origin: null,
      originDetail: null,
    },
    commands: { canCancel: true, retry: null, retryTitle: "Retry" },
    supportIds: null,
    promptHint: null,
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
    ...overrides,
  };
  return value;
}

const running = row("job-1", "Fox", [{ kind: "cancel", isEnabled: true, help: "Cancel" }]);

const failed = row("job-2", "Cat", [{ kind: "retry" }], {
  commands: { canCancel: false, retry: "retry", retryTitle: "Retry" },
  supportIds: "request_id=req-1 correlation_id=none",
});
failed.model = {
  ...failed.model,
  statusLabel: "Failed",
  statusRole: "danger",
  isWorking: false,
  stage: null,
  elapsed: null,
  note: { text: "The model is unavailable", role: "danger" },
};

const paused = row("job-3", "Owl", [{ kind: "confirmPrice" }], {
  priceConfirmation: {
    title: "Start this job for $9.50?",
    message:
      "This is the price from Higgsfield for the uploaded files. It takes today's spending over your cap.",
    confirmTitle: "Start for $9.50",
  },
});

const snapshot: QueueSnapshot_Serialize = {
  ...emptyQueue,
  activeCount: 2,
  slotsSummary: "1 of 4 slots busy",
  canPauseAll: true,
  active: [
    { kind: "job", row: running },
    { kind: "job", row: paused },
  ],
  problems: [{ kind: "job", row: failed }],
};

afterEach(() => {
  useToastStore.setState({ queuedNotice: null });
});

describe("QueuePage", () => {
  it("has nothing to pause or resume when empty, and goes to Create", async () => {
    const { router } = await renderRoute("/queue");
    expect(await screen.findByRole("button", { name: "Resume all" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Pause all" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Go to Create" }));
    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/create");
    });
  });

  it("lists the jobs and runs their commands", async () => {
    const { calls } = await renderRoute("/queue", { queue_snapshot: () => snapshot });
    expect(await screen.findByText("1 of 4 slots busy")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    fireEvent.click(screen.getByRole("button", { name: "Pause all" }));
    await waitFor(() => {
      expect(calls).toContainEqual({
        command: "queue_job_command",
        args: { id: "job-1", command: "cancel" },
      });
      expect(calls.some((call) => call.command === "queue_pause_all")).toBe(true);
    });
  });

  it("asks before resuming a job above its estimate", async () => {
    const { calls } = await renderRoute("/queue", { queue_snapshot: () => snapshot });
    fireEvent.click(await screen.findByRole("button", { name: "Review price" }));
    const dialog = screen.getByRole("alertdialog", { name: "Start this job for $9.50?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Start for $9.50" }));
    await waitFor(() => {
      expect(calls).toContainEqual({
        command: "queue_job_command",
        args: { id: "job-3", command: "resume" },
      });
    });
  });

  it("removes and clears the jobs that didn't finish", async () => {
    const { calls } = await renderRoute("/queue", { queue_snapshot: () => snapshot });
    const problems = await screen.findByRole("listbox", { name: "Jobs that didn't finish" });
    fireEvent.click(within(problems).getByRole("button", { name: "Remove" }));
    fireEvent.click(screen.getByRole("button", { name: "Clear all" }));
    const dialog = screen.getByRole("alertdialog", { name: "Clear the jobs that didn't finish?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Clear all" }));
    await waitFor(() => {
      expect(calls).toContainEqual({ command: "queue_remove_problem", args: { id: "job-2" } });
      expect(calls.some((call) => call.command === "queue_clear_problems")).toBe(true);
    });
  });

  it("duplicates a job, asking first when it goes over the cap", async () => {
    let approvals: unknown[] = [];
    const { calls } = await renderRoute("/queue", {
      queue_snapshot: () => snapshot,
      queue_duplicate: ({ overCapApproved }) => {
        approvals = [...approvals, overCapApproved];
        return overCapApproved
          ? { kind: "queued", notice: "Final queued · $0.68" }
          : {
              kind: "confirm",
              confirmation: {
                title: "Over today's spend cap",
                message: "This job adds $0.68.",
                confirmTitle: "Start anyway",
              },
            };
      },
    });
    const problems = await screen.findByRole("listbox", { name: "Jobs that didn't finish" });
    fireEvent.click(within(problems).getByRole("button", { name: "Duplicate" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Over today's spend cap" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Start anyway" }));
    await waitFor(() => {
      expect(useToastStore.getState().queuedNotice?.text).toBe("Final queued · $0.68");
    });
    expect(approvals).toEqual([false, true]);
    expect(calls.filter((call) => call.command === "queue_duplicate")).toHaveLength(2);
  });

  it("selects the row of the job an agent's show asked for", async () => {
    await renderRoute("/create", { queue_snapshot: () => snapshot });
    await emitEvent("agent-show-requested", {
      route: { page: "queue", job: "job-3", item: null, sheet: null },
    });
    const owl = await screen.findByRole("option", { name: "Owl" });
    await waitFor(() => {
      expect(owl).toHaveAttribute("aria-selected", "true");
    });
    expect(screen.getByRole("option", { name: "Fox" })).toHaveAttribute("aria-selected", "false");
  });

  it("cancels and retries the selected row from the keyboard", async () => {
    const { calls } = await renderRoute("/queue", { queue_snapshot: () => snapshot });
    const fox = await screen.findByRole("option", { name: "Fox" });
    fireEvent.click(fox);
    expect(fox).toHaveAttribute("aria-selected", "true");
    await waitFor(() => {
      fireEvent.keyDown(window, { key: ".", code: "Period", ctrlKey: true });
      expect(calls).toContainEqual({
        command: "queue_job_command",
        args: { id: "job-1", command: "cancel" },
      });
    });
    fireEvent.click(screen.getByRole("option", { name: "Cat" }));
    await waitFor(() => {
      fireEvent.keyDown(window, { key: "R", code: "KeyR", ctrlKey: true, shiftKey: true });
      expect(calls).toContainEqual({ command: "queue_job_command", args: { id: "job-2", command: "retry" } });
    });
  });

  it("titles Queue › Retry Job after the selected row's button", async () => {
    const download = row("job-4", "Hare", [{ kind: "retryDownload" }], {
      commands: { canCancel: false, retry: "retry", retryTitle: "Retry Download" },
    });
    const { calls } = await renderRoute("/queue", {
      queue_snapshot: () => ({ ...snapshot, problems: [{ kind: "job", row: download }] }),
    });
    fireEvent.click(await screen.findByRole("option", { name: "Hare" }));
    await waitFor(() => {
      const published = calls.filter((call) => call.command === "app_menu_publish").at(-1);
      expect(published?.args.commands).toEqual(
        expect.arrayContaining([{ command: "retryJob", title: "Retry Download" }]),
      );
    });
  });
});
