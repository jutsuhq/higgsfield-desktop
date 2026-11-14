import type { EventCallback } from "@tauri-apps/api/event";
import { act, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { useAppEvent, type AppEvent } from "./events";

/** An event whose `listen` resolves when the test says so. */
function fakeEvent<T>() {
  const stop = vi.fn();
  let callback: EventCallback<T> | undefined;
  let resolve: (() => void) | undefined;
  const event: AppEvent<T> = {
    listen: (cb) => {
      callback = cb;
      return new Promise((done) => {
        resolve = () => {
          done(stop);
        };
      });
    },
  };
  const send = (payload: T) => {
    callback?.({ event: "test", id: 1, payload });
  };
  return { event, stop, send, resolve: () => resolve?.() };
}

function Listener<T>({ event, onPayload }: { event: AppEvent<T>; onPayload: (payload: T) => void }) {
  useAppEvent(event, onPayload);
  return null;
}

describe("useAppEvent", () => {
  it("passes payloads on and unlistens on unmount", async () => {
    const fake = fakeEvent<number>();
    const onPayload = vi.fn();
    const { unmount } = render(<Listener event={fake.event} onPayload={onPayload} />);
    await act(async () => {
      fake.resolve();
      await Promise.resolve();
    });
    fake.send(7);
    expect(onPayload).toHaveBeenCalledWith(7);
    unmount();
    expect(fake.stop).toHaveBeenCalledOnce();
  });

  it("unlistens when unmounted before listen resolves", async () => {
    const fake = fakeEvent<number>();
    const { unmount } = render(<Listener event={fake.event} onPayload={vi.fn()} />);
    unmount();
    expect(fake.stop).not.toHaveBeenCalled();
    await act(async () => {
      fake.resolve();
      await Promise.resolve();
    });
    expect(fake.stop).toHaveBeenCalledOnce();
  });
});
