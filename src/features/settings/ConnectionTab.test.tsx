import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { ConnectionDetails_Serialize, ConnectionSnapshot_Serialize } from "../../bindings";
import { connectedWithKey } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";

const account: ConnectionSnapshot_Serialize = {
  ...connectedWithKey,
  kind: "account",
  needsAttention: true,
  plate: {
    badge: "Creator Plus",
    tier: "elite",
    amount: "1,250",
    unit: "credits",
    warning: null,
    isAPIKey: false,
    accessibilityLabel: "Creator Plus plan, 1,250 credits left",
  },
  signOut: {
    title: "Sign out of Higgsfield?",
    message:
      "Your sign-in is deleted from Windows Credential Manager. Queued jobs pause until you connect again.",
    confirmTitle: "Sign Out",
  },
};

const accountDetails: ConnectionDetails_Serialize = {
  rows: [
    { label: "Account", value: "ada@example.com", isMonospaced: false },
    { label: "Plan", value: "Creator Plus", isMonospaced: false },
  ],
  unlimitedText: "Seedance 2.5",
  usesUnlimited: true,
};

const commandsOf = (calls: { command: string }[]) => calls.map((call) => call.command);

describe("Settings › Connection", () => {
  it("removes the key only after confirming", async () => {
    const { calls } = await renderRoute("/settings");
    fireEvent.click(await screen.findByRole("button", { name: "Remove" }));
    let dialog = await screen.findByRole("alertdialog", { name: "Remove the API key?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(commandsOf(calls)).not.toContain("sign_out");

    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    dialog = await screen.findByRole("alertdialog", { name: "Remove the API key?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Remove" }));
    await waitFor(() => {
      expect(commandsOf(calls)).toContain("sign_out");
    });
  });

  it("writes an account's unlimited switch and signs in again", async () => {
    const { calls } = await renderRoute("/settings", {
      connection_status: () => account,
      connection_details: () => accountDetails,
    });
    expect(await screen.findByText("ada@example.com")).toBeInTheDocument();

    const toggle = screen.getByRole("switch", { name: "Use unlimited generations" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    fireEvent.click(toggle);
    await waitFor(() => {
      expect(calls).toContainEqual({
        command: "set_preferences",
        args: { patch: { "settings.useUnlimited": false } },
      });
    });

    fireEvent.click(screen.getByRole("button", { name: "Sign In Again" }));
    await waitFor(() => {
      expect(commandsOf(calls)).toContain("start_sign_in");
    });
  });
});
