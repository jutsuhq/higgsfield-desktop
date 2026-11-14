import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import type { About, MenuCommandRequested_Serialize, SidebarLayout } from "../bindings";
import { AboutPage } from "../features/about/AboutPage";
import { createQueryClient } from "../lib/queryClient";
import { runPageCommand } from "../lib/shortcuts/pageCommands";
import { emitEvent, mockCommands, sidebarShown } from "../lib/testing/ipcMock";
import { renderRoute } from "../test/renderRoute";

const menuCommand = (payload: MenuCommandRequested_Serialize) => emitEvent("menu-command-requested", payload);

describe("the window's menu", () => {
  it("is told which commands the page on screen carries out", async () => {
    const { calls } = await renderRoute("/library");
    await waitFor(() => {
      const published = calls.filter((call) => call.command === "app_menu_publish").at(-1);
      const names = (published?.args.commands as { command: string }[] | undefined)?.map(
        (state) => state.command,
      );
      expect(names).toEqual(
        expect.arrayContaining(["create", "queue", "library", "settings", "open", "toggleSidebar"]),
      );
    });
  });

  it("runs the command its item names, as the shortcut does", async () => {
    await renderRoute("/create");
    await waitFor(async () => {
      await menuCommand({ command: "queue", template: null });
      expect(screen.getByRole("heading", { level: 1, name: "Queue" })).toBeInTheDocument();
    });
    await menuCommand({ command: "settings", template: null });
    expect(await screen.findByRole("heading", { level: 1, name: "Settings" })).toBeInTheDocument();
  });

  it("opens Create with a template and applies it", async () => {
    const { calls } = await renderRoute("/queue", { templates_apply: () => null });
    await menuCommand({ command: null, template: "00000000-0000-0000-0000-000000000001" });
    expect(await screen.findByRole("heading", { level: 1, name: "Create" })).toBeInTheDocument();
    await waitFor(() => {
      expect(calls.some((call) => call.command === "templates_apply")).toBe(true);
    });
  });
});

describe("the sidebar", () => {
  it("hides with its toggle and keeps that", async () => {
    let layout: SidebarLayout = sidebarShown;
    const { calls } = await renderRoute("/create", {
      sidebar_layout: () => layout,
      set_sidebar_layout: (args) => {
        layout = { ...layout, isCollapsed: args.isCollapsed as boolean };
        return layout;
      },
    });
    fireEvent.click(await screen.findByRole("button", { name: "Hide Sidebar" }));
    await waitFor(() => {
      expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
    });
    expect(calls.find((call) => call.command === "set_sidebar_layout")?.args).toEqual({
      isCollapsed: true,
      width: null,
    });
  });

  it("hides and shows with Toggle Sidebar from the menu", async () => {
    await renderRoute("/create", {
      set_sidebar_layout: (args) => ({ ...sidebarShown, isCollapsed: args.isCollapsed }),
    });
    await screen.findByRole("navigation");
    await waitFor(() => {
      runPageCommand("toggleSidebar");
      expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
    });
  });

  it("keeps the width its edge is dragged to, within Rust's bounds", async () => {
    const { calls } = await renderRoute("/create", {
      set_sidebar_layout: (args) => ({ ...sidebarShown, width: args.width }),
    });
    const edge = await screen.findByRole("separator", { name: "Sidebar width" });
    edge.setPointerCapture = () => undefined;
    fireEvent.pointerDown(edge, { clientX: 228, pointerId: 1 });
    fireEvent.pointerMove(edge, { clientX: 600, pointerId: 1 });
    expect(edge).toHaveAttribute("aria-valuenow", "320");
    fireEvent.pointerUp(edge, { pointerId: 1 });
    await waitFor(() => {
      expect(calls.find((call) => call.command === "set_sidebar_layout")?.args).toEqual({
        isCollapsed: null,
        width: 320,
      });
    });
  });
});

describe("files opened from outside", () => {
  it("go to Create, and its alert says what was left out", async () => {
    let alert: unknown = { title: "Can't open these files", message: "notes.txt isn't a video or an image." };
    const { router } = await renderRoute("/library", {
      opened_files_take: () => {
        const next = alert;
        alert = null;
        return next;
      },
    });
    await emitEvent("files-opened", { count: 1 });
    expect(await screen.findByText("notes.txt isn't a video or an image.")).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/create");
    fireEvent.click(screen.getByRole("button", { name: "OK" }));
    await waitFor(() => {
      expect(screen.queryByText("notes.txt isn't a video or an image.")).not.toBeInTheDocument();
    });
  });
});

describe("Delete from Library while text is edited", () => {
  it("deletes the word before the cursor instead", () => {
    function Field() {
      const [value, setValue] = useState("a slow pan");
      return (
        <input
          aria-label="Prompt"
          value={value}
          onChange={(event) => {
            setValue(event.target.value);
          }}
        />
      );
    }
    render(<Field />);
    const field = screen.getByRole("textbox", { name: "Prompt" });
    field.focus();
    (field as HTMLInputElement).setSelectionRange(10, 10);
    runPageCommand("deleteResult");
    expect(field).toHaveValue("a slow ");
  });
});

describe("the About window", () => {
  it("opens the repository and shows the third-party notices", async () => {
    const about: About = {
      version: "Version 0.1.0",
      disclaimer: "Not affiliated with Higgsfield.",
      credits: ["Video by FFmpeg, GNU Lesser General Public License 2.1 or later."],
      repository: "https://github.com/jutsuhq/jutsu-studio",
      repositoryTitle: "github.com/jutsuhq/jutsu-studio",
      noticesTitle: "Third-Party Notices",
      notices: "# Third-party notices",
    };
    const calls = mockCommands({ about_content: () => about });
    render(
      <QueryClientProvider client={createQueryClient()}>
        <AboutPage />
      </QueryClientProvider>,
    );
    expect(await screen.findByText("Version 0.1.0")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "github.com/jutsuhq/jutsu-studio" }));
    expect(calls.some((call) => call.command === "about_open_repository")).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Third-Party Notices" }));
    expect(await screen.findByText("# Third-party notices")).toBeInTheDocument();
  });
});
