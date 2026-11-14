import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect } from "react";

import { commands, events } from "../bindings";
import { showError, useCreateNotice } from "../features/create/createData";
import { useAppEvent } from "../lib/events";
import { queryKeys } from "../lib/queryKeys";
import { runPageCommand, usePublishedCommands } from "../lib/shortcuts/pageCommands";

/**
 * The window's menu (`AppCommands.swift`): tells Rust which commands the page on screen carries out, so it enables
 * their items; runs an item chosen there with the page's own handler; and applies a template from File › New from
 * Template as Create's Templates menu does, then shows Create.
 */
export function useAppMenu(): void {
  const published = usePublishedCommands();
  const navigate = useNavigate();
  const client = useQueryClient();

  useEffect(() => {
    void commands.appMenuPublish(published).catch(() => undefined);
  }, [published]);

  useAppEvent(events.menuCommandRequested, ({ command, template }) => {
    if (command) runPageCommand(command);
    if (!template) return;
    void navigate({ to: "/create" });
    commands.templatesApply(template).then(
      (notice) => {
        if (notice) useCreateNotice.getState().show(notice);
        void client.invalidateQueries({ queryKey: queryKeys.create.all });
        void client.invalidateQueries({ queryKey: queryKeys.images.all });
      },
      (error: unknown) => {
        showError("Can't open the template", error);
      },
    );
  });
}
