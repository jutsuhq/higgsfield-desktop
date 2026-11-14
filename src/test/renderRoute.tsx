import { QueryClientProvider } from "@tanstack/react-query";
import { createMemoryHistory, createRouter, RouterProvider } from "@tanstack/react-router";
import { render, screen } from "@testing-library/react";

import { routeTree } from "../app/routes";
import { createQueryClient } from "../lib/queryClient";
import { mockCommands, type CommandHandlers } from "../lib/testing/ipcMock";

/**
 * Renders the app's routes at `path` over mocked IPC (a connected API key unless `commands` says otherwise);
 * resolves once a page title is on screen. `calls` lists the commands the window sent.
 */
export async function renderRoute(path: string, commands: CommandHandlers = {}) {
  const calls = mockCommands(commands);
  const router = createRouter({ routeTree, history: createMemoryHistory({ initialEntries: [path] }) });
  const result = render(
    <QueryClientProvider client={createQueryClient()}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  await screen.findByRole("heading", { level: 1 });
  return { ...result, router, calls };
}
