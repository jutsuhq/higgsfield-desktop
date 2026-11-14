import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { CreditHistoryView_Serialize, SpendingView_Serialize } from "../../bindings";
import { renderRoute } from "../../test/renderRoute";
import { internalError } from "./settingsTesting";

const apiKeyView: SpendingView_Serialize = {
  periods: [
    { value: "today", title: "Today" },
    { value: "week", title: "7 days" },
    { value: "month", title: "30 days" },
  ],
  period: "today",
  charged: "$10.00",
  chargedDetail: "1 job",
  notCharged: "$4.00",
  notChargedDetail: "Failed, rejected or canceled · 1 job",
  spentFooter: "Estimates made before each job was submitted.",
  breakdowns: [
    { title: "By mode", lines: [{ title: "Motion transfer", jobsText: "1 job", total: "$10.00" }] },
  ],
  cap: {
    isEnabled: true,
    amountText: "$75.00",
    committedText: "$12.50",
    committedError: null,
    footer: "A soft cap on the API balance.",
  },
  lowCredits: null,
  showsCreditHistory: false,
  refreshAfterMs: 3_600_000,
};

const accountView: SpendingView_Serialize = {
  ...apiKeyView,
  cap: {
    ...apiKeyView.cap,
    isEnabled: false,
    amountText: "1,000 credits",
    committedText: "Unavailable",
    committedError: "disk",
  },
  lowCredits: "50 credits",
  showsCreditHistory: true,
};

const row = (amount: string, index: number) => ({
  title: `Job ${String(index)}`,
  detail: `Spent · Oct 13, 2025 at 12:0${String(index)}`,
  amount,
  isAdded: amount.startsWith("+"),
});

async function openSpending(handlers: Parameters<typeof renderRoute>[1]) {
  const rendered = await renderRoute("/settings", handlers);
  fireEvent.click(await screen.findByRole("button", { name: "Spending" }));
  return rendered;
}

describe("Settings › Spending", () => {
  it("switches the period and commits the cap on Return", async () => {
    const { calls } = await openSpending({
      spending_view: () => apiKeyView,
      spending_commit_cap: () => "$120.00",
    });
    fireEvent.click(await screen.findByRole("button", { name: "7 days" }));
    await waitFor(() => {
      expect(calls).toContainEqual({ command: "spending_view", args: { period: "week" } });
    });

    const amount = screen.getByRole("textbox", { name: "Amount" });
    fireEvent.change(amount, { target: { value: "120" } });
    fireEvent.keyDown(amount, { key: "Enter" });
    await waitFor(() => {
      expect(amount).toHaveValue("$120.00");
    });
    expect(calls).toContainEqual({ command: "spending_commit_cap", args: { text: "120" } });

    fireEvent.click(screen.getByRole("switch", { name: "Daily cap" }));
    await waitFor(() => {
      expect(calls).toContainEqual({ command: "spending_set_cap_enabled", args: { isOn: false } });
    });
  });

  it("says why the CSV can't be exported", async () => {
    await openSpending({
      spending_view: () => apiKeyView,
      spending_export_csv: internalError("The disk is full."),
    });
    fireEvent.click(await screen.findByRole("button", { name: "Export CSV…" }));
    const alert = await screen.findByRole("alertdialog");
    expect(within(alert).getByText("Can't export")).toBeInTheDocument();
    expect(within(alert).getByText("The disk is full.")).toBeInTheDocument();
  });

  it("lists an account's credit history ten at a time", async () => {
    const first: CreditHistoryView_Serialize = {
      rows: [row("−45", 1), row("+39", 2)],
      failure: null,
      isLoading: false,
      hasOlder: true,
      isEmpty: false,
    };
    const more: CreditHistoryView_Serialize = {
      ...first,
      rows: [...first.rows, row("−5", 3)],
      hasOlder: false,
    };
    const { calls } = await openSpending({
      spending_view: () => accountView,
      credit_history: () => first,
      credit_history_more: () => more,
    });
    expect(await screen.findByText("Job 1")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Amount" })).toBeDisabled();
    expect(screen.getByRole("textbox", { name: "Warn below" })).toHaveValue("50 credits");

    fireEvent.click(screen.getByRole("button", { name: "Show More" }));
    expect(await screen.findByText("Job 3")).toBeInTheDocument();
    expect(calls.some((call) => call.command === "credit_history_more")).toBe(true);
    expect(screen.queryByRole("button", { name: "Show More" })).not.toBeInTheDocument();
  });

  it("offers Try Again after a failed page", async () => {
    let view: CreditHistoryView_Serialize = {
      rows: [],
      failure: { title: "Can't load the credit history", message: "Try again.", action: null },
      isLoading: false,
      hasOlder: false,
      isEmpty: false,
    };
    await openSpending({
      spending_view: () => accountView,
      credit_history: () => view,
      credit_history_retry: () => {
        view = { rows: [], failure: null, isLoading: false, hasOlder: false, isEmpty: true };
        return view;
      },
    });
    fireEvent.click(await screen.findByRole("button", { name: "Try Again" }));
    expect(await screen.findByText("No credit movements yet")).toBeInTheDocument();
  });
});
