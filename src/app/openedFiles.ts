import { useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";

import { commands, events, type OpenedFilesAlert } from "../bindings";
import { showError } from "../features/create/createData";
import { useAppEvent } from "../lib/events";

/** File › Open…: videos and images, which Create takes once it is on screen. */
export function chooseFilesToOpen(): void {
  commands.openedFilesChoose().catch((error: unknown) => {
    showError("Can't open the files", error);
  });
}

/**
 * Files opened from outside or with File › Open… (`NavigationModel.open`, `CreateView.takePendingOpens`): the window
 * goes to Create, and Create takes them once it is on screen with a connection. Answers with the alert Rust gave, if
 * any, and how to dismiss it.
 */
export function useOpenedFiles(isCreateShown: boolean): {
  alert: OpenedFilesAlert | null;
  dismiss: () => void;
} {
  const navigate = useNavigate();
  const [alert, setAlert] = useState<OpenedFilesAlert | null>(null);
  const [opened, setOpened] = useState(0);

  useAppEvent(events.filesOpened, () => {
    setOpened((count) => count + 1);
    void navigate({ to: "/create" });
  });

  useEffect(() => {
    if (!isCreateShown) return;
    // What was taken is shown even if the page moved on meanwhile: nothing waits for it any more.
    commands.openedFilesTake().then(
      (shown) => {
        if (shown) setAlert(shown);
      },
      (error: unknown) => {
        showError("Can't open the files", error);
      },
    );
  }, [isCreateShown, opened]);

  return {
    alert,
    dismiss: () => {
      setAlert(null);
    },
  };
}
