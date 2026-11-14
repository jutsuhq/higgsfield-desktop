import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type {
  EditingExportMenu,
  EnhanceChoice_Serialize,
  EnhanceMenu_Serialize,
  EnhanceSheet_Serialize,
  OnDeviceSheet_Serialize,
} from "../../bindings";
import { renderRoute } from "../../test/renderRoute";
import { defaultFilter, useLibraryView } from "./libraryData";
import { resultDetail } from "./libraryTesting";

const choice: EnhanceChoice_Serialize = {
  kind: "upscale",
  target: "1080p",
  engine: "standard",
  frameRate: 30,
  preset: "aigc",
  aspect: "9:16",
  language: "eng",
  voice: null,
};

const sheet: EnhanceSheet_Serialize = {
  choice,
  title: "Upscale to 1080p",
  subtitle: "rooftop · Final",
  options: {
    engines: [
      { value: "standard", title: "Standard" },
      { value: "topaz", title: "Topaz" },
    ],
    targets: [
      { value: "1080p", title: "1080p" },
      { value: "4k", title: "4K" },
    ],
    takesFrameRate: true,
    frameRates: [
      { value: 30, title: "30 fps" },
      { value: 60, title: "60 fps" },
    ],
    presets: [{ value: "aigc", title: "AI video" }],
    aspects: [],
    languages: [],
  },
  canvas: null,
  canvasCaption: "",
  isTooLongToReframe: false,
  priceText: "~$0.40 on the API balance",
  needsConsent: false,
  priceTag: "$0.40",
  canQueue: true,
};

const menu: EditingExportMenu = {
  presets: [
    { preset: "premiere", title: "Premiere Pro (ProRes 422 · MOV)" },
    { preset: "davinciResolve", title: "DaVinci Resolve (ProRes 422 · MOV)" },
  ],
  isEnabled: true,
  writesSidecar: true,
  hasSubtitles: false,
  writesSubtitles: false,
  isExporting: false,
};

const remoteMenu: EnhanceMenu_Serialize = {
  disabledReason: null,
  hint: "Upscale, reframe or dub on Higgsfield",
  isDisabled: false,
  showsRemote: true,
  smoothMotion: false,
  upscaleOnDevice: false,
  upscaleSubmenu: false,
  busy: null,
};

const detail = resultDetail("final", {
  title: "rooftop · Final",
  enhance: remoteMenu,
});

afterEach(() => {
  useLibraryView.setState({
    origin: "local",
    kind: "all",
    filter: defaultFilter,
    expandedGroups: [],
    order: [],
  });
});

describe("Enhance", () => {
  it("prices the operation, asks over the cap and queues it approved", async () => {
    let approvals = 0;
    const { calls } = await renderRoute("/library/result/final", {
      library_detail: () => detail,
      editing_export_menu: () => menu,
      enhance_choice: () => choice,
      enhance_sheet: (args: Record<string, unknown>) => ({ ...sheet, choice: args.choice }),
      enhance_queue: (args: Record<string, unknown>) => {
        approvals += 1;
        return args.approved
          ? { kind: "queued", notice: "Upscale queued · ~$0.40" }
          : {
              kind: "confirm",
              confirmation: {
                title: "Over today's cap",
                message: "This takes today to $80.",
                confirmTitle: "Start Anyway",
              },
            };
      },
    });
    fireEvent.click(await screen.findByRole("button", { name: "Enhance" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Upscale…" }));
    const dialog = await screen.findByRole("dialog", { name: "Upscale to 1080p" });
    expect(within(dialog).getByText("~$0.40 on the API balance")).toBeInTheDocument();

    fireEvent.click(within(dialog).getByRole("button", { name: "60 fps" }));
    await waitFor(() => {
      expect(
        calls.some(
          (call) =>
            call.command === "enhance_sheet" &&
            (call.args.choice as EnhanceChoice_Serialize).frameRate === 60,
        ),
      ).toBe(true);
    });

    // Apply waits for the new options' price.
    await waitFor(() => {
      expect(within(dialog).getByRole("button", { name: /Upscale to 1080p/ })).toBeEnabled();
    });
    fireEvent.click(within(dialog).getByRole("button", { name: /Upscale to 1080p/ }));
    const question = await screen.findByRole("alertdialog", { name: "Over today's cap" });
    fireEvent.click(within(question).getByRole("button", { name: "Start Anyway" }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "Upscale to 1080p" })).not.toBeInTheDocument();
    });
    expect(approvals).toBe(2);
    expect(
      calls.filter((call) => call.command === "enhance_queue").map((call) => call.args.approved),
    ).toEqual([false, true]);
  });

  it("clears the price and the unknown-price consent while the new options are priced, then queues them", async () => {
    const pending: { release: (() => void) | null } = { release: null };
    const { calls } = await renderRoute("/library/result/final", {
      library_detail: () => detail,
      editing_export_menu: () => menu,
      enhance_choice: () => choice,
      enhance_sheet: (args: Record<string, unknown>) => {
        const asked = args.choice as EnhanceChoice_Serialize;
        if (asked.frameRate === 30) {
          return { ...sheet, choice: asked, priceText: "Price unknown", priceTag: null, needsConsent: true };
        }
        return new Promise((resolve) => {
          pending.release = () => {
            resolve({ ...sheet, choice: asked, priceText: "~$0.80 on the API balance", priceTag: "$0.80" });
          };
        });
      },
      enhance_queue: () => ({ kind: "queued", notice: "Upscale queued · ~$0.80" }),
    });
    fireEvent.click(await screen.findByRole("button", { name: "Enhance" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Upscale…" }));
    const dialog = await screen.findByRole("dialog", { name: "Upscale to 1080p" });
    fireEvent.click(
      await within(dialog).findByRole("checkbox", { name: /Charge whatever Higgsfield bills/ }),
    );
    expect(within(dialog).getByRole("button", { name: /Upscale to 1080p/ })).toBeEnabled();

    fireEvent.click(within(dialog).getByRole("button", { name: "60 fps" }));
    expect(await within(dialog).findByText("Asking Higgsfield for the price…")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: /Upscale to 1080p/ })).toBeDisabled();
    expect(within(dialog).queryByRole("checkbox")).not.toBeInTheDocument();

    await waitFor(() => {
      expect(pending.release).not.toBeNull();
    });
    pending.release?.();
    expect(await within(dialog).findByText("~$0.80 on the API balance")).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: /Upscale to 1080p/ }));
    await waitFor(() => {
      expect(calls.some((call) => call.command === "enhance_queue")).toBe(true);
    });
    const queued = calls.find((call) => call.command === "enhance_queue");
    expect((queued?.args.choice as EnhanceChoice_Serialize).frameRate).toBe(60);
  });

  it("asks for the unknown-price consent again after a re-quote", async () => {
    await renderRoute("/library/result/final", {
      library_detail: () => detail,
      editing_export_menu: () => menu,
      enhance_choice: () => choice,
      enhance_sheet: (args: Record<string, unknown>) => ({
        ...sheet,
        choice: args.choice,
        priceText: "Price unknown",
        priceTag: null,
        needsConsent: true,
      }),
    });
    fireEvent.click(await screen.findByRole("button", { name: "Enhance" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Upscale…" }));
    const dialog = await screen.findByRole("dialog", { name: "Upscale to 1080p" });
    fireEvent.click(
      await within(dialog).findByRole("checkbox", { name: /Charge whatever Higgsfield bills/ }),
    );
    fireEvent.click(within(dialog).getByRole("button", { name: "60 fps" }));
    await waitFor(() => {
      expect(
        within(dialog).getByRole("checkbox", { name: /Charge whatever Higgsfield bills/ }),
      ).not.toBeChecked();
    });
    expect(within(dialog).getByRole("button", { name: /Upscale to 1080p/ })).toBeDisabled();
  });

  it("lists every item off with the reason", async () => {
    await renderRoute("/library/result/final", {
      library_detail: () => ({
        ...detail,
        enhance: { ...remoteMenu, disabledReason: "Connect a Higgsfield account or API key", hint: "" },
      }),
      editing_export_menu: () => menu,
    });
    fireEvent.click(await screen.findByRole("button", { name: "Enhance" }));
    expect(screen.getByRole("menuitem", { name: "Connect a Higgsfield account or API key" })).toBeDisabled();
    expect(screen.getByRole("menuitem", { name: "Dub…" })).toBeDisabled();
  });
});

describe("Enhance on this computer", () => {
  const localMenu: EnhanceMenu_Serialize = { ...remoteMenu, smoothMotion: true, upscaleSubmenu: true };
  const smoothSheet: OnDeviceSheet_Serialize = {
    kind: "smoothMotion",
    title: "Smooth Motion",
    itemTitle: "rooftop · Final",
    motions: [
      { value: "fps60", title: "60 fps" },
      { value: "slowMotion2x", title: "2× Slow Motion" },
    ],
    motion: "fps60",
    sizes: null,
    optionNotes: [{ text: "New frames are made between the clip's own.", isWarning: false }],
    modelNotice: null,
    consentTitle: "Download the model",
    notes: [{ text: "Free · runs on this computer", isWarning: false }],
    unavailableReason: null,
    startTitle: "Smooth Motion to 60 fps",
    priceTag: "Free",
    canStart: true,
  };
  const upscaleSheet: OnDeviceSheet_Serialize = {
    ...smoothSheet,
    kind: "upscaleVideo",
    title: "Upscale on This Computer",
    motions: [],
    motion: null,
    sizes: "4× · 854×480 → 3416×1920",
    optionNotes: [],
    modelNotice:
      "Upscaling on this computer needs Real-ESRGAN, which Jutsu Studio downloads from its GitHub release once, about 52 MB.",
    startTitle: "Upscale 4×",
  };

  it("offers Smooth Motion and both Upscales, and starts the chosen motion", async () => {
    const { calls } = await renderRoute("/library/result/final", {
      library_detail: () => ({ ...detail, enhance: localMenu }),
      editing_export_menu: () => menu,
      on_device_sheet: (args: Record<string, unknown>) =>
        args.motion === "slowMotion2x"
          ? { ...smoothSheet, motion: "slowMotion2x", startTitle: "2× Slow Motion" }
          : smoothSheet,
      on_device_start: () => ({ kind: "started", runId: "run-1" }),
    });
    fireEvent.click(await screen.findByRole("button", { name: "Enhance" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Upscale" }));
    expect(screen.getByRole("menuitem", { name: "With Higgsfield…" })).toBeEnabled();
    expect(screen.getByRole("menuitem", { name: "On This Computer…" })).toBeEnabled();
    fireEvent.click(screen.getByRole("menuitem", { name: "Smooth Motion…" }));
    const dialog = await screen.findByRole("dialog", { name: "Smooth Motion" });
    expect(within(dialog).getByText("Free · runs on this computer")).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "2× Slow Motion" }));
    fireEvent.click(await within(dialog).findByRole("button", { name: /2× Slow Motion.*Free/ }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "Smooth Motion" })).not.toBeInTheDocument();
    });
    expect(calls.find((call) => call.command === "on_device_start")?.args).toEqual({
      id: "final",
      kind: "smoothMotion",
      motion: "slowMotion2x",
      agrees: false,
    });
  });

  it("starts the first upscale only once the download is agreed to", async () => {
    const { calls } = await renderRoute("/library/result/final", {
      library_detail: () => ({ ...detail, enhance: localMenu }),
      editing_export_menu: () => menu,
      on_device_sheet: () => upscaleSheet,
      on_device_start: () => ({ kind: "started", runId: "run-1" }),
    });
    fireEvent.click(await screen.findByRole("button", { name: "Enhance" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Upscale" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "On This Computer…" }));
    const dialog = await screen.findByRole("dialog", { name: "Upscale on This Computer" });
    expect(within(dialog).getByText(/about 52 MB/)).toBeInTheDocument();
    const start = within(dialog).getByRole("button", { name: /Upscale 4×/ });
    expect(start).toBeDisabled();
    fireEvent.click(within(dialog).getByRole("checkbox", { name: "Download the model" }));
    expect(start).toBeEnabled();
    fireEvent.click(start);
    await waitFor(() => {
      expect(calls.find((call) => call.command === "on_device_start")?.args.agrees).toBe(true);
    });
  });

  it("turns this computer's items off while a run on the item goes on, and shows it with its Cancel", async () => {
    const { calls } = await renderRoute("/library/result/final", {
      library_detail: () => ({
        ...detail,
        enhance: { ...localMenu, upscaleSubmenu: false, busy: "Wait until Upscale 4× finishes" },
      }),
      editing_export_menu: () => menu,
      on_device_runs: () => [
        { id: "run-1", itemId: "final", title: "Upscale 4×", progress: "Upscale 4× · 42%", fraction: 0.42 },
      ],
    });
    expect(await screen.findByText("Upscale 4× · 42%")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Enhance" }));
    expect(screen.getByRole("menuitem", { name: "Wait until Upscale 4× finishes" })).toBeDisabled();
    expect(screen.getByRole("menuitem", { name: "Smooth Motion…" })).toBeDisabled();
    expect(screen.getByRole("menuitem", { name: "Upscale…" })).toBeEnabled();
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => {
      expect(calls).toContainEqual({ command: "on_device_cancel", args: { runId: "run-1" } });
    });
  });
});

describe("Export for Editing", () => {
  it("offers the presets and the sidecar toggle, and exports with the picked preset", async () => {
    const { calls } = await renderRoute("/library/result/final", {
      library_detail: () => detail,
      editing_export_menu: () => menu,
      editing_export: () => true,
    });
    fireEvent.click(await screen.findByRole("button", { name: "Export for Editing…" }));
    expect(screen.getByRole("menuitemcheckbox", { name: /Include JSON Sidecar/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(screen.queryByRole("menuitemcheckbox", { name: /Include Subtitles/ })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("menuitemcheckbox", { name: /Include JSON Sidecar/ }));
    await waitFor(() => {
      expect(calls).toContainEqual({
        command: "set_preferences",
        args: { patch: { "export.writesSidecar": false } },
      });
    });

    fireEvent.click(screen.getByRole("button", { name: "Export for Editing…" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "DaVinci Resolve (ProRes 422 · MOV)" }));
    await waitFor(() => {
      expect(
        calls.some((call) => call.command === "editing_export" && call.args.preset === "davinciResolve"),
      ).toBe(true);
    });
  });
});
