import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { disconnected, emitConnection } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";

const signingIn = (url: string | null) => ({
  ...disconnected,
  activity: { kind: "signingIn" as const, url },
});
const sheetName = "Finish signing in in your browser";

async function openSheet(url: string | null) {
  const rendered = await renderRoute("/onboarding", { connection_status: () => disconnected });
  await emitConnection(signingIn(url));
  const sheet = await screen.findByRole("dialog", { name: sheetName });
  return { ...rendered, sheet };
}

const commandsOf = (calls: { command: string }[]) => calls.map((call) => call.command);

describe("SignInSheet", () => {
  it("has no link buttons until the page is known", async () => {
    const { sheet } = await openSheet(null);
    expect(within(sheet).queryByRole("button", { name: "Open Again" })).not.toBeInTheDocument();
    expect(within(sheet).queryByRole("button", { name: "Copy Link" })).not.toBeInTheDocument();
  });

  it("opens the page again and copies its link", async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const { sheet, calls } = await openSheet("https://auth.example/authorize?state=s");

    fireEvent.click(within(sheet).getByRole("button", { name: "Open Again" }));
    await waitFor(() => {
      expect(commandsOf(calls)).toContain("open_sign_in_page");
    });

    fireEvent.click(within(sheet).getByRole("button", { name: "Copy Link" }));
    expect(await within(sheet).findByRole("button", { name: "Copied" })).toBeInTheDocument();
    expect(writeText).toHaveBeenCalledWith("https://auth.example/authorize?state=s");
  });

  it("cancels from the button and from Esc", async () => {
    const { sheet, calls } = await openSheet("https://auth.example/authorize?state=s");
    fireEvent.click(within(sheet).getByRole("button", { name: "Cancel" }));
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => {
      expect(commandsOf(calls).filter((command) => command === "cancel_sign_in")).toHaveLength(2);
    });
  });

  it("closes once the sign-in is over", async () => {
    await openSheet(null);
    await emitConnection(disconnected);
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: sheetName })).not.toBeInTheDocument();
    });
  });
});
