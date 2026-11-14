import { act, fireEvent, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { renderRoute } from "../test/renderRoute";
import { draftReplacementDurationMs, queuedNoticeDurationMs, useToastStore } from "./toastStore";

afterEach(() => {
  act(() => {
    useToastStore.setState({ queuedNotice: null, draftReplacement: null });
  });
  vi.useRealTimers();
});

describe("ToastHost", () => {
  it("shows what was queued with a way to the Queue, which dismisses it", async () => {
    const { router } = await renderRoute("/create");
    act(() => {
      useToastStore.getState().showQueued("Queued 2 jobs");
    });
    expect(screen.getByRole("status")).toHaveTextContent("Queued 2 jobs");
    fireEvent.click(screen.getByRole("button", { name: "View in Queue" }));
    expect(await screen.findByRole("heading", { level: 1, name: "Queue" })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/queue");
    expect(screen.getByRole("status")).toBeEmptyDOMElement();
  });

  it("offers no way to the Queue on the Queue", async () => {
    await renderRoute("/queue");
    act(() => {
      useToastStore.getState().showQueued("Queued 1 job");
    });
    expect(screen.queryByRole("button", { name: "View in Queue" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.getByRole("status")).toBeEmptyDOMElement();
  });

  it("goes away after its time unless a newer toast replaced it", async () => {
    await renderRoute("/create");
    vi.useFakeTimers();
    act(() => {
      useToastStore.getState().showQueued("First");
    });
    act(() => {
      vi.advanceTimersByTime(queuedNoticeDurationMs - 1000);
      useToastStore.getState().showQueued("Second");
    });
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(screen.getByRole("status")).toHaveTextContent("Second");
    act(() => {
      vi.advanceTimersByTime(queuedNoticeDurationMs);
    });
    expect(screen.getByRole("status")).toBeEmptyDOMElement();
  });

  it("offers a replaced draft back with Undo", async () => {
    await renderRoute("/create");
    vi.useFakeTimers();
    const undo = vi.fn();
    act(() => {
      useToastStore.getState().showDraftReplaced(undo);
    });
    expect(screen.getByRole("status")).toHaveTextContent("Draft replaced");
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(undo).toHaveBeenCalledOnce();
    expect(screen.getByRole("status")).toBeEmptyDOMElement();

    act(() => {
      useToastStore.getState().showDraftReplaced(undo);
    });
    act(() => {
      vi.advanceTimersByTime(draftReplacementDurationMs);
    });
    expect(screen.getByRole("status")).toBeEmptyDOMElement();
  });

  it("drops a replaced draft's toast when a submit queues", () => {
    useToastStore.getState().showDraftReplaced(vi.fn());
    useToastStore.getState().showQueued("Queued 1 job");
    expect(useToastStore.getState().draftReplacement).toBeNull();
  });
});
