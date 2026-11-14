import { QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type {
  CharacterSnapshot_Serialize,
  CharactersPanel_Serialize,
  ImageGenerateOutcome,
} from "../../bindings";
import { createQueryClient } from "../../lib/queryClient";
import { mockCommands, type CommandHandlers } from "../../lib/testing/ipcMock";
import { useCreateNotice } from "./createData";
import { CharacterSheetsPicker, CreateCharacterSheet } from "./CreateCharacter";

const form: CharacterSnapshot_Serialize = {
  connected: true,
  presets: {
    tiles: [
      {
        id: "p1",
        name: "Knight",
        previewUrl: "https://example.com/p1.png",
        isPicked: false,
        enabled: true,
        hint: "",
      },
    ],
    hasMore: false,
    failure: null,
    showsClear: false,
  },
  kind: "human",
  kinds: [
    { value: "human", title: "Person" },
    { value: "animal", title: "Animal" },
  ],
  tier: "standard",
  tiers: [],
  brief: "",
  briefDetail: null,
  face: null,
  styles: [],
  canAddStyle: true,
  randomize: true,
  count: 1,
  counts: [1, 2, 3, 4],
  traits: {
    detail: "Random",
    failure: null,
    groups: [
      {
        id: "hair",
        title: "Hair",
        choice: "Random",
        choiceKind: "random",
        limit: null,
        accessibilityValue: "Random",
        traits: [
          { id: "red", title: "Red", isPicked: false },
          { id: "black", title: "Black", isPicked: false },
        ],
      },
    ],
  },
  runs: [],
  generate: { isBusy: false, enabled: true, price: "1.125 cr", hint: "1.125 cr · Price from Higgsfield" },
  notice: null,
  closeHint: "Close",
};

function renderWith(node: React.ReactNode, handlers: CommandHandlers) {
  const calls = mockCommands(handlers);
  render(<QueryClientProvider client={createQueryClient()}>{node}</QueryClientProvider>);
  return calls;
}

function editsOf(calls: { command: string; args: Record<string, unknown> }[]) {
  return calls.filter((call) => call.command === "characters_edit").map((call) => call.args.edit);
}

afterEach(() => {
  useCreateNotice.getState().dismiss();
});

describe("Create Character", () => {
  it("sends the form's edits to Rust", async () => {
    const calls = renderWith(<CreateCharacterSheet onClose={() => undefined} />, {
      characters_snapshot: () => form,
      characters_edit: () => form,
    });
    fireEvent.click(await screen.findByRole("button", { name: "Animal" }));
    fireEvent.change(screen.getByLabelText("Description"), { target: { value: "A tall knight" } });
    fireEvent.click(screen.getByRole("button", { name: "Knight" }));
    fireEvent.click(screen.getByRole("switch", { name: "Randomize" }));
    fireEvent.click(screen.getByRole("button", { name: "3" }));
    fireEvent.click(screen.getByRole("button", { name: "Hair" }));
    fireEvent.click(screen.getByRole("button", { name: "Red" }));
    expect(editsOf(calls)).toEqual([
      { kind: "selectKind", value: "animal" },
      { kind: "setBrief", text: "A tall knight" },
      { kind: "applyPreset", id: "p1" },
      { kind: "setRandomize", on: false },
      { kind: "setCount", count: 3 },
      { kind: "toggleTrait", group: "hair", id: "red" },
    ]);
  });

  it("chooses the face and style images through the dialog in Rust", async () => {
    const calls = renderWith(<CreateCharacterSheet onClose={() => undefined} />, {
      characters_snapshot: () => form,
      characters_choose_images: () => form,
    });
    fireEvent.click(await screen.findByRole("button", { name: "Choose a face" }));
    fireEvent.click(screen.getByRole("button", { name: "Choose style images" }));
    await waitFor(() => {
      expect(
        calls.filter((call) => call.command === "characters_choose_images").map((call) => call.args.styles),
      ).toEqual([false, true]);
    });
  });

  it("asks before going over the cap, then generates approved", async () => {
    const answers: ImageGenerateOutcome[] = [
      {
        kind: "confirm",
        confirmation: {
          title: "Go over today's cap?",
          message: "This takes today's spending to 51 cr of your 50 cr cap.",
          confirmTitle: "Generate Anyway",
        },
      },
      { kind: "started" },
    ];
    const calls = renderWith(<CreateCharacterSheet onClose={() => undefined} />, {
      characters_snapshot: () => form,
      characters_generate: () => answers.shift(),
    });
    fireEvent.click(await screen.findByRole("button", { name: /Generate/ }));
    await waitFor(() => {
      expect(useCreateNotice.getState().confirmation?.question.title).toBe("Go over today's cap?");
    });
    useCreateNotice.getState().confirmation?.approve();
    await waitFor(() => {
      expect(
        calls.filter((call) => call.command === "characters_generate").map((call) => call.args.approved),
      ).toEqual([false, true]);
    });
  });
});

describe("References › From Higgsfield › Characters", () => {
  const panel: CharactersPanel_Serialize = {
    title: "Characters",
    detail: "Character sheets of your account. Pick one to use it as a reference image.",
    pending: 1,
    tiles: [{ id: "sheet-1", path: null, url: "https://example.com/sheet.png" }],
    listsAccount: false,
    note: "Only the characters made in Jutsu Studio are listed. Sheets made on higgsfield.ai aren't shown yet.",
    failure: null,
    hasMore: false,
    isEmpty: false,
    fullHint: null,
  };

  it("adds a picked sheet as a reference", async () => {
    let done = false;
    const calls = renderWith(
      <CharacterSheetsPicker
        panel={panel}
        adding={null}
        onLoad={() => undefined}
        onClose={() => undefined}
        onDone={() => {
          done = true;
        }}
      />,
      { create_add_character: () => null },
    );
    fireEvent.click(screen.getByRole("button", { name: "Character sheet" }));
    await waitFor(() => {
      expect(done).toBe(true);
    });
    expect(calls.find((call) => call.command === "create_add_character")?.args.id).toBe("sheet-1");
  });

  it("says why a sheet wasn't added", async () => {
    renderWith(
      <CharacterSheetsPicker
        panel={panel}
        adding={null}
        onLoad={() => undefined}
        onClose={() => undefined}
        onDone={() => undefined}
      />,
      { create_add_character: () => "This character sheet is no longer listed." },
    );
    fireEvent.click(screen.getByRole("button", { name: "Character sheet" }));
    expect(await screen.findByText("This character sheet is no longer listed.")).toBeTruthy();
  });
});
