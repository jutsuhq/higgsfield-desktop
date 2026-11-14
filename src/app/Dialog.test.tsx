import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { WindowSheet } from "../bindings";
import { mockCommands } from "../lib/testing/ipcMock";
import { Dialog } from "./Dialog";

function Sheets({ open }: { open: WindowSheet[] }) {
  return (
    <>
      {open.map((sheet) => (
        <Dialog key={sheet} title={sheet} sheet={sheet} onCancel={() => undefined}>
          {sheet}
        </Dialog>
      ))}
    </>
  );
}

describe("Dialog", () => {
  it("tells Rust the sheet over the window, the outermost one, until none is open", () => {
    const calls = mockCommands();
    const reported = () =>
      calls.filter((call) => call.command === "agents_set_window_sheet").map((call) => call.args.sheet);

    const { rerender, unmount } = render(<Sheets open={["subtitles"]} />);
    expect(reported().at(-1)).toBe("subtitles");
    rerender(<Sheets open={["subtitles", "other"]} />);
    expect(reported().at(-1)).toBe("subtitles");
    rerender(<Sheets open={["other"]} />);
    expect(reported().at(-1)).toBe("other");
    unmount();
    expect(reported().at(-1)).toBeNull();
  });
});
