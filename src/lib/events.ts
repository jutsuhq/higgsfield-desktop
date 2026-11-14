import { useQueryClient, type QueryClient, type QueryKey } from "@tanstack/react-query";
import type { EventCallback, UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";

/** The part of a generated event (`events.connectionChanged`) the hooks use. */
export interface AppEvent<T> {
  listen: (callback: EventCallback<T>) => Promise<UnlistenFn>;
}

/**
 * Calls `handler` with every payload of `event` while the component is mounted. Unlistens on unmount, also when the
 * component unmounts before `listen` has resolved.
 */
export function useAppEvent<T>(event: AppEvent<T>, handler: (payload: T) => void): void {
  const latest = useRef(handler);
  useEffect(() => {
    latest.current = handler;
  });
  useEffect(() => {
    let isDisposed = false;
    let unlisten: UnlistenFn | undefined;
    void event
      .listen((message) => {
        latest.current(message.payload);
      })
      .then((stop) => {
        if (isDisposed) stop();
        else unlisten = stop;
      });
    return () => {
      isDisposed = true;
      unlisten?.();
    };
  }, [event]);
}

/**
 * Keeps queries in step with Rust: each `event` invalidates `keys`, after `update` has written what the payload
 * already says straight into the cache.
 */
export function useInvalidateOn<T>(
  event: AppEvent<T>,
  keys: readonly QueryKey[],
  update?: (client: QueryClient, payload: T) => void,
): void {
  const client = useQueryClient();
  useAppEvent(event, (payload) => {
    update?.(client, payload);
    for (const queryKey of keys) void client.invalidateQueries({ queryKey });
  });
}
