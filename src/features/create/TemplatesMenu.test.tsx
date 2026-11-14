import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { TemplateRow, TemplateSaveOutcome } from "../../bindings";
import { useCreateNotice } from "./createData";
import { renderRoute } from "../../test/renderRoute";

const templates: TemplateRow[] = [
  { id: "t-1", name: "Fox dance", summary: "Motion · Kling 3.0 · 2 references", side: "video" },
  { id: "t-2", name: "Portraits", summary: "Image · Soul 2", side: "image" },
];

afterEach(() => {
  useCreateNotice.getState().dismiss();
});

async function openMenu() {
  fireEvent.click(await screen.findByRole("button", { name: "Templates" }));
  return screen.findByRole("menu");
}

describe("TemplatesMenu", () => {
  it("says there are none yet and can't manage them", async () => {
    await renderRoute("/create");
    const menu = await openMenu();
    expect(await within(menu).findByRole("menuitem", { name: "No templates yet" })).toBeTruthy();
    expect(within(menu).getByRole("menuitem", { name: "Manage Templates…" })).toBeDisabled();
  });

  it("applies a template and shows what it reports", async () => {
    const { calls } = await renderRoute("/create", {
      templates_list: () => templates,
      templates_apply: () => ({ title: "Template opened on another model", detail: "Kling 3.0 isn't offered." }),
    });
    const menu = await openMenu();
    fireEvent.click(await within(menu).findByRole("menuitem", { name: "Portraits" }));
    expect(await screen.findByRole("alertdialog", { name: "Template opened on another model" })).toBeTruthy();
    expect(calls.find((call) => call.command === "templates_apply")?.args.id).toBe("t-2");
  });

  it("saves the side in front under the suggested name, and says why it couldn't", async () => {
    const { calls } = await renderRoute("/create", {
      templates_suggested_name: () => "Motion – A fox",
      templates_save: (): TemplateSaveOutcome => ({
        kind: "failed",
        notice: { title: "Can't save the template", detail: "The disk is full." },
      }),
    });
    const menu = await openMenu();
    fireEvent.click(within(menu).getByRole("menuitem", { name: "Save as Template…" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Save as Template" });
    const field = within(dialog).getByLabelText("Name");
    expect((field as HTMLInputElement).value).toBe("Motion – A fox");
    fireEvent.change(field, { target: { value: "Fox" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alertdialog", { name: "Can't save the template" })).toBeTruthy();
    expect(calls.find((call) => call.command === "templates_save")?.args).toEqual({ side: "video", name: "Fox" });
  });

  it("renames and deletes templates from Manage Templates", async () => {
    const { calls } = await renderRoute("/create", {
      templates_list: () => templates,
      templates_rename: () => "Fox dance 2",
    });
    const menu = await openMenu();
    fireEvent.click(await within(menu).findByRole("menuitem", { name: "Manage Templates…" }));
    const sheet = await screen.findByRole("dialog", { name: "Templates" });
    expect(within(sheet).getByText("Image · Soul 2")).toBeTruthy();

    fireEvent.click(within(sheet).getAllByRole("button", { name: "Rename" })[0] as HTMLElement);
    const naming = await screen.findByRole("alertdialog", { name: "Rename Template" });
    fireEvent.change(within(naming).getByLabelText("Name"), { target: { value: "Fox dance 2" } });
    fireEvent.click(within(naming).getByRole("button", { name: "Rename" }));

    fireEvent.click(within(sheet).getAllByRole("button", { name: "Delete" })[1] as HTMLElement);
    const question = await screen.findByRole("alertdialog", { name: "Delete “Portraits”?" });
    fireEvent.click(within(question).getByRole("button", { name: "Delete Template" }));
    await waitFor(() => {
      expect(calls.map((call) => call.command)).toContain("templates_delete");
    });
    expect(calls.find((call) => call.command === "templates_rename")?.args).toEqual({ id: "t-1", name: "Fox dance 2" });
    expect(calls.find((call) => call.command === "templates_delete")?.args.id).toBe("t-2");
  });
});
