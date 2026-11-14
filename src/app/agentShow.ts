import { useNavigate } from "@tanstack/react-router";
import { create } from "zustand";

import { events, type AgentShowRoute } from "../bindings";
import { useAppEvent } from "../lib/events";

/**
 * Where an agent's `show` takes the window (AgentService+Show.swift): a page, a job's row in the Queue, a job's result
 * in Library, or an item's page in Library. Rust already brought the window forward.
 */
export function agentShowPath(route: AgentShowRoute): string {
  if (route.page === "library" && route.item) return `/library/result/${route.item}`;
  if (route.page === "library" && route.job) return `/library/result/${route.job}`;
  return `/${route.page}`;
}

/** The Queue row `show` asked for (`NavigationModel.queueSelection`): the Queue selects it, then clears it. */
export const useShownQueueJob = create<{ id: string | null; set: (id: string | null) => void }>((set) => ({
  id: null,
  set: (id) => {
    set({ id });
  },
}));

/** The sheet `show` asked to open on an item (`upscale_on_mac` or `subtitles`): its page opens it, then clears it. */
export const useShownSheet = create<{
  item: string | null;
  sheet: string | null;
  set: (item: string | null, sheet: string | null) => void;
}>((set) => ({
  item: null,
  sheet: null,
  set: (item, sheet) => {
    set({ item, sheet });
  },
}));

export function useAgentShow(): void {
  const navigate = useNavigate();
  useAppEvent(events.agentShowRequested, ({ route }) => {
    if (route.page === "queue" && route.job) useShownQueueJob.getState().set(route.job);
    if (route.item && route.sheet) useShownSheet.getState().set(route.item, route.sheet);
    void navigate({ to: agentShowPath(route) });
  });
}
