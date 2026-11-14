import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import { FilterMenu, SearchField } from "./Pickers";

const modes = [
  { value: null, title: "Any mode" },
  { value: "motion", title: "Motion" },
];

function Menu() {
  const [mode, setMode] = useState<string | null>(null);
  return <FilterMenu title="Mode" value={mode} options={modes} onChange={setMode} />;
}

describe("FilterMenu", () => {
  it("shows the choice and closes once one is picked", () => {
    render(<Menu />);
    const button = screen.getByRole("button", { name: "Mode" });
    expect(button).toHaveAccessibleDescription("Any mode");

    fireEvent.click(button);
    expect(button).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("menuitemradio", { name: "Any mode" })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(screen.getByRole("menuitemradio", { name: "Motion" }));

    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(button).toHaveAccessibleDescription("Motion");
  });

  it("closes on Escape without choosing", () => {
    render(<Menu />);
    fireEvent.click(screen.getByRole("button", { name: "Mode" }));
    fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Mode" })).toHaveAccessibleDescription("Any mode");
  });
});

function Search() {
  const [text, setText] = useState("");
  return <SearchField value={text} onChange={setText} prompt="Search prompts and names" />;
}

describe("SearchField", () => {
  it("offers Clear search only with text, and Escape clears", () => {
    render(<Search />);
    const field = screen.getByRole("textbox", { name: "Search prompts and names" });
    expect(screen.queryByRole("button", { name: "Clear search" })).not.toBeInTheDocument();
    fireEvent.change(field, { target: { value: "neon" } });
    fireEvent.click(screen.getByRole("button", { name: "Clear search" }));
    expect(field).toHaveValue("");
    fireEvent.change(field, { target: { value: "neon" } });
    fireEvent.keyDown(field, { key: "Escape" });
    expect(field).toHaveValue("");
  });

  it("takes the focus on Ctrl+F", () => {
    render(<Search />);
    fireEvent.keyDown(window, { key: "f", ctrlKey: true });
    expect(screen.getByRole("textbox")).toHaveFocus();
  });
});
