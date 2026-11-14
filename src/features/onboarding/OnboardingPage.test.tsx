import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { disconnected, emitConnection } from "../../lib/testing/ipcMock";
import { renderRoute } from "../../test/renderRoute";

const renderOnboarding = () => renderRoute("/onboarding", { connection_status: () => disconnected });

describe("Onboarding", () => {
  it("starts the account sign-in", async () => {
    const { calls } = await renderOnboarding();
    fireEvent.click(await screen.findByRole("button", { name: "Connect your Higgsfield account" }));
    await waitFor(() => {
      expect(calls.map((call) => call.command)).toContain("start_sign_in");
    });
  });

  it("connects a key once both fields are filled, with Enter moving from Key ID to Secret", async () => {
    const { calls } = await renderOnboarding();
    const connect = await screen.findByRole("button", { name: "Connect key" });
    expect(connect).toBeDisabled();
    const keyId = screen.getByRole("textbox", { name: "Key ID" });
    const secret = screen.getByLabelText("Secret");
    fireEvent.change(keyId, { target: { value: "kid" } });
    expect(connect).toBeDisabled();
    fireEvent.keyDown(keyId, { key: "Enter" });
    expect(secret).toHaveFocus();
    fireEvent.change(secret, { target: { value: "   " } });
    expect(connect).toBeDisabled();
    fireEvent.change(secret, { target: { value: "s3cret" } });
    expect(connect).toBeEnabled();
    fireEvent.keyDown(secret, { key: "Enter" });
    await waitFor(() => {
      expect(calls).toContainEqual({ command: "connect_api_key", args: { keyId: "kid", secret: "s3cret" } });
    });
  });

  it("disables both ways and shows progress while the key is checked", async () => {
    await renderOnboarding();
    await screen.findByRole("button", { name: "Connect key" });
    await emitConnection({ ...disconnected, activity: { kind: "validatingKey" } });
    const connect = screen.getByRole("button", { name: "Connect key" });
    await waitFor(() => {
      expect(within(connect).getByRole("progressbar")).toBeInTheDocument();
    });
    expect(connect).toBeDisabled();
    expect(screen.getByRole("button", { name: "Connect your Higgsfield account" })).toBeDisabled();
  });

  it("opens the console from the link", async () => {
    const { calls } = await renderOnboarding();
    fireEvent.click(await screen.findByRole("button", { name: "Create a key in the Higgsfield console" }));
    await waitFor(() => {
      expect(calls.map((call) => call.command)).toContain("open_higgsfield_console");
    });
  });

  it("shows the connection's alert with its action, and OK dismisses it", async () => {
    const { calls } = await renderOnboarding();
    await emitConnection({
      ...disconnected,
      alert: {
        title: "API key is invalid",
        message: "Check the key ID and secret in the Higgsfield console.",
        action: "openConsole",
      },
    });
    const alert = await screen.findByRole("alertdialog", { name: "API key is invalid" });
    expect(within(alert).getByRole("button", { name: "Open Console" })).toBeInTheDocument();
    fireEvent.click(within(alert).getByRole("button", { name: "OK" }));
    await waitFor(() => {
      expect(calls.map((call) => call.command)).toContain("dismiss_connection_alert");
    });
  });
});
