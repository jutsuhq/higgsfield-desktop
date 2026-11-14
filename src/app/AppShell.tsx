import { Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { ArrowLeft } from "lucide-react";
import { useCallback, useRef } from "react";

import { commands } from "../bindings";
import { Spinner } from "../design/Spinner";
import { Button } from "../design/Styles";
import { usePageCommand } from "../lib/shortcuts/pageCommands";
import {
  quitBinding,
  settingsBinding,
  toggleSidebarBinding,
  useKeyBinding,
  useShortcut,
} from "../lib/shortcuts/useShortcut";
import { useAgentShow } from "./agentShow";
import { useAppMenu } from "./appMenu";
import { AlertDialog } from "./Dialog";
import { useConnection } from "./liveData";
import { chooseFilesToOpen, useOpenedFiles } from "./openedFiles";
import { screenFor, screens, type Screen, type ScreenId } from "./screens";
import { Sidebar, useSidebarLayout } from "./Sidebar";
import { ToastHost } from "./ToastHost";

/**
 * The main window once connected (RootView.swift): the sidebar beside the page, the toast over the page.
 * The shortcuts of Settings › Shortcuts (Ctrl+1/2/3 by default) open Create, Queue and Library, and Ctrl+, opens
 * Settings, as ⌘ does on macOS; File › Open… (Ctrl+O) opens videos and images from any page, and Ctrl+Shift+S shows
 * or hides the sidebar. Each also runs from the window's menu.
 * Without a connection only Settings is reachable, from onboarding: it shows alone, with Back.
 */
export function AppShell() {
  const navigate = useNavigate();
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const nav = useRef<HTMLElement>(null);
  const selection = screenFor(pathname)?.id;
  const connection = useConnection();
  // While the stored connection is read at launch the window shows neither onboarding nor the sidebar.
  const isRestoring = connection?.isRestoring === true;
  const isDisconnected = connection?.kind === null && !isRestoring;
  const sidebar = useSidebarLayout();

  const open = useCallback(
    (screen: Screen) => {
      void navigate({ to: screen.path });
    },
    [navigate],
  );
  /** Opens the screen `id`; the shortcuts work only once connected. */
  const opener = (id: ScreenId) => () => {
    const screen = screens.find((candidate) => candidate.id === id);
    if (screen) open(screen);
  };
  const shortcutsOn = !isDisconnected && !isRestoring;
  useShortcut("create", opener("create"), shortcutsOn);
  useShortcut("queue", opener("queue"), shortcutsOn);
  useShortcut("library", opener("library"), shortcutsOn);
  useShortcut("open", chooseFilesToOpen, shortcutsOn);
  useKeyBinding(settingsBinding, opener("settings"), shortcutsOn);
  usePageCommand("settings", opener("settings"), shortcutsOn);
  useKeyBinding(toggleSidebarBinding, sidebar.toggle, shortcutsOn);
  usePageCommand("toggleSidebar", sidebar.toggle, shortcutsOn);
  useKeyBinding(quitBinding, () => void commands.trayQuit());
  useAgentShow();
  useAppMenu();
  const opened = useOpenedFiles(shortcutsOn && pathname === "/create");

  if (isRestoring) {
    return (
      <div className="flex h-full items-center justify-center bg-background text-muted">
        <Spinner label="Checking the connection" />
      </div>
    );
  }

  if (isDisconnected) {
    return (
      <div className="relative flex h-full flex-col bg-background text-text">
        <div className="px-page pt-small">
          <Button
            variant="secondary"
            title="Back"
            icon={ArrowLeft}
            onClick={() => {
              void navigate({ to: "/onboarding" });
            }}
          />
        </div>
        <main className="flex min-h-0 flex-1 flex-col overflow-auto">
          <Outlet />
        </main>
      </div>
    );
  }

  return (
    <div className="relative flex h-full bg-background text-text">
      <Sidebar selection={selection} onSelect={open} navRef={nav} layout={sidebar} />
      <div className="relative flex min-w-0 flex-1 flex-col">
        <main className="flex min-h-0 flex-1 flex-col overflow-auto">
          <Outlet />
        </main>
        <ToastHost />
      </div>
      {opened.alert && (
        <AlertDialog title={opened.alert.title} message={opened.alert.message} onCancel={opened.dismiss}>
          <Button variant="primary" title="OK" onClick={opened.dismiss} />
        </AlertDialog>
      )}
    </div>
  );
}
