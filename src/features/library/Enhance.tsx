import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Pause, Play, Sparkles } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import {
  commands,
  type EnhanceChoice_Serialize,
  type EnhanceMenu_Serialize,
  type MadeFromLink_Serialize,
  type OnDeviceKind,
  type PostActionKind,
  type ReframeCanvas,
  type VoiceRow_Serialize,
} from "../../bindings";
import { AlertDialog, Dialog } from "../../app/Dialog";
import { useToastStore } from "../../app/toastStore";
import { SectionLabel } from "../../design/Components";
import { FilterMenu, SegmentedControl } from "../../design/Pickers";
import { Spinner } from "../../design/Spinner";
import { Button } from "../../design/Styles";
import { queryKeys } from "../../lib/queryKeys";
import { errorText } from "../create/createData";
import { ContextMenu, type MenuItem } from "./LibraryDialogs";
import { OnDeviceSheet } from "./OnDeviceSheet";

/** The cloud items of the Enhance menu, in `EnhanceMenu.swift`'s order. */
const ITEMS: (readonly [PostActionKind, string] | "divider")[] = [
  ["upscale", "Upscale…"],
  ["reframe", "Reframe…"],
  ["removeBackground", "Remove Background…"],
  "divider",
  ["dub", "Dub…"],
  ["changeVoice", "Change Voice…"],
];

/**
 * The Enhance menu of a result's page (`EnhanceMenu.swift`). The on-device items (Smooth Motion…, Upscale on This
 * Mac…) come with the on-device engines; until then it offers only Higgsfield's operations.
 */
export function EnhanceMenu({ id, menu }: { id: string; menu: EnhanceMenu_Serialize }) {
  const button = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<{ x: number; y: number } | null>(null);
  const [kind, setKind] = useState<PostActionKind | null>(null);
  const [local, setLocal] = useState<OnDeviceKind | null>(null);
  const reason = menu.disabledReason;
  const heading = (text: string): MenuItem => ({ title: text, isDisabled: true, run: () => undefined });
  const openLocal = (target: OnDeviceKind) => () => {
    setLocal(target);
  };
  const localItems: MenuItem[] = [
    ...(menu.smoothMotion ? [{ title: "Smooth Motion…", run: openLocal("smoothMotion") }] : []),
    ...(menu.upscaleOnDevice ? [{ title: "Upscale on This Computer…", run: openLocal("upscaleVideo") }] : []),
  ].map((item) => ({ ...item, isDisabled: menu.busy !== null }));
  const remote: (MenuItem | "divider")[] = ITEMS.flatMap((item): (MenuItem | "divider")[] => {
    if (item === "divider") return [item];
    const open = {
      isDisabled: reason !== null,
      run: () => {
        setKind(item[0]);
      },
    };
    if (item[0] === "upscale" && menu.upscaleSubmenu) {
      return [
        {
          title: "Upscale",
          run: () => undefined,
          children: [
            { title: "With Higgsfield…", ...open },
            { title: "On This Computer…", run: openLocal("upscaleVideo") },
          ],
        },
      ];
    }
    return [{ title: item[1], ...open }];
  });
  const items: (MenuItem | "divider")[] = [
    ...(menu.busy !== null && localItems.length > 0 ? [heading(menu.busy)] : []),
    ...localItems,
    ...(menu.showsRemote && localItems.length > 0 ? ["divider" as const] : []),
    ...(menu.showsRemote && reason !== null ? [heading(reason)] : []),
    ...(menu.showsRemote ? remote : []),
  ];
  return (
    <>
      <div ref={button}>
        <Button
          variant="secondary"
          icon={Sparkles}
          title="Enhance"
          className="w-full"
          hint={menu.hint}
          aria-haspopup="menu"
          disabled={menu.isDisabled}
          onClick={() => {
            const rect = button.current?.getBoundingClientRect();
            setPosition({ x: rect?.left ?? 0, y: rect?.bottom ?? 0 });
          }}
        />
      </div>
      {position && (
        <ContextMenu
          x={position.x}
          y={position.y}
          items={items}
          onClose={() => {
            setPosition(null);
          }}
        />
      )}
      {local && (
        <OnDeviceSheet
          id={id}
          kind={local}
          onClose={() => {
            setLocal(null);
          }}
        />
      )}
      {kind && (
        <EnhanceSheet
          id={id}
          kind={kind}
          onClose={() => {
            setKind(null);
          }}
        />
      )}
    </>
  );
}

/** "Upscaled from rooftop" on a result an operation made; opens the one it was made from. */
export function MadeFromLink({ link }: { link: MadeFromLink_Serialize }) {
  const navigate = useNavigate();
  const parent = link.parentId;
  return (
    <Button
      variant="textLink"
      title={link.title}
      disabled={parent === null}
      onClick={() => {
        if (parent !== null) void navigate({ to: "/library/result/$id", params: { id: parent } });
      }}
    />
  );
}

/** The Enhance sheet of one operation (`EnhanceSheet.swift`): its options, the price, then the job in the Queue. */
export function EnhanceSheet({
  id,
  kind,
  onClose,
}: {
  id: string;
  kind: PostActionKind;
  onClose: () => void;
}) {
  const [choice, setChoice] = useState<EnhanceChoice_Serialize | null>(null);
  const [consent, setConsent] = useState(false);
  const [isBusy, setIsBusy] = useState(false);
  const [question, setQuestion] = useState<{ title: string; message: string; confirmTitle?: string } | null>(
    null,
  );
  useEffect(() => {
    void commands.enhanceChoice(kind).then(setChoice);
  }, [kind]);
  const query = useQuery({
    queryKey: [...queryKeys.library.all, "enhance", id, choice],
    queryFn: () => (choice ? commands.enhanceSheet(id, choice) : Promise.resolve(null)),
    placeholderData: keepPreviousData,
    enabled: choice !== null,
  });
  const sheet = query.data;
  // The previous options’ sheet stays on screen while the current ones are priced, without its price.
  const isPricing = query.isPlaceholderData;
  const shown = isPricing && choice ? choice : sheet?.choice;
  // Every re-quote clears the price and the consent to an unknown one (`EnhanceSheet.quote`).
  const update = (patch: Partial<EnhanceChoice_Serialize>) => {
    if (!choice) return;
    setConsent(false);
    setChoice({ ...choice, ...patch });
  };
  const submit = (approved: boolean) => {
    if (!sheet || isPricing) return;
    setIsBusy(true);
    void commands
      .enhanceQueue(id, sheet.choice, approved)
      .then((outcome) => {
        switch (outcome.kind) {
          case "queued":
            useToastStore.getState().showQueued(outcome.notice);
            onClose();
            return;
          case "confirm":
            setQuestion(outcome.confirmation);
            return;
          case "blocked":
            setQuestion({ title: "Can't queue this", message: outcome.reason });
            return;
          case "busy":
          case "failed":
            return;
        }
      })
      .finally(() => {
        setIsBusy(false);
      });
  };
  const options = sheet?.options;
  return (
    <Dialog title={sheet?.title ?? "Enhance"} onCancel={onClose} className="w-480">
      <div className="flex flex-col gap-6">
        <p className="m-0 text-heading font-bold">{sheet?.title ?? ""}</p>
        {sheet && <p className="m-0 text-body text-muted">{sheet.subtitle}</p>}
      </div>
      {!sheet || !options || !choice ? (
        <Spinner />
      ) : (
        <>
          {kind === "upscale" && (
            <div className="flex flex-col gap-small">
              <SectionLabel title="Engine" />
              <SegmentedControl
                isCompact
                value={shown?.engine ?? sheet.choice.engine}
                options={options.engines}
                onChange={(engine) => {
                  update({ engine });
                }}
              />
              <SectionLabel title="Target" />
              <SegmentedControl
                isCompact
                value={shown?.target ?? sheet.choice.target}
                options={options.targets}
                onChange={(target) => {
                  update({ target });
                }}
              />
              {options.takesFrameRate && (
                <>
                  <SectionLabel title="Frame rate" />
                  <SegmentedControl
                    isCompact
                    value={shown?.frameRate ?? sheet.choice.frameRate}
                    options={options.frameRates}
                    onChange={(frameRate) => {
                      update({ frameRate });
                    }}
                  />
                  <p className="m-0 text-caption text-muted">60 fps costs twice as much.</p>
                  <FilterMenu
                    title="Tuned for"
                    value={shown?.preset ?? sheet.choice.preset}
                    options={options.presets}
                    onChange={(preset) => {
                      update({ preset });
                    }}
                  />
                </>
              )}
            </div>
          )}
          {kind === "reframe" && (
            <div className="flex flex-col gap-small">
              <SectionLabel title="Canvas" />
              <SegmentedControl
                isCompact
                value={shown?.aspect ?? sheet.choice.aspect}
                options={options.aspects}
                onChange={(aspect) => {
                  update({ aspect });
                }}
              />
              {sheet.canvas && <ReframePreview canvas={sheet.canvas} />}
              <p className="m-0 text-caption text-muted">{sheet.canvasCaption}</p>
              {sheet.isTooLongToReframe && (
                <p className="m-0 text-caption text-warning">Reframe takes results up to a minute long.</p>
              )}
            </div>
          )}
          {kind === "dub" && (
            <div className="flex flex-col gap-small">
              <FilterMenu
                title="Language"
                value={shown?.language ?? sheet.choice.language}
                options={options.languages}
                onChange={(language) => {
                  update({ language });
                }}
              />
              <p className="m-0 text-caption text-muted">
                The speech is translated and lip-synced; the picture stays as it is.
              </p>
            </div>
          )}
          {kind === "changeVoice" && (
            <VoicePicker
              selected={(shown ?? sheet.choice).voice?.id ?? null}
              onPick={(voice) => {
                update({ voice });
              }}
            />
          )}
          {kind === "removeBackground" && (
            <p className="m-0 text-caption text-muted">
              The subject stays on a black background, in an MP4 without transparency. The clip&apos;s sound
              is put back.
            </p>
          )}
          <div className="flex flex-col gap-6">
            <p className="m-0 text-caption text-muted">
              {isPricing ? "Asking Higgsfield for the price…" : sheet.priceText}
            </p>
            {!isPricing && sheet.needsConsent && (
              <label className="flex items-center gap-6 text-body">
                <input
                  type="checkbox"
                  checked={consent}
                  onChange={(event) => {
                    setConsent(event.target.checked);
                  }}
                />
                Charge whatever Higgsfield bills when it finishes
              </label>
            )}
            <p className="m-0 text-caption text-muted">
              The result is added to the Library, linked to this one.
            </p>
          </div>
          <div className="flex justify-end gap-small">
            <Button variant="secondary" title="Cancel" onClick={onClose} />
            <Button
              variant="primary"
              title={sheet.title}
              price={isPricing ? undefined : (sheet.priceTag ?? undefined)}
              disabled={isPricing || !sheet.canQueue || isBusy || (sheet.needsConsent && !consent)}
              onClick={() => {
                submit(false);
              }}
            />
          </div>
        </>
      )}
      {question && (
        <AlertDialog
          title={question.title}
          message={question.message}
          onCancel={() => {
            setQuestion(null);
          }}
        >
          <Button
            variant="secondary"
            title={question.confirmTitle ? "Cancel" : "OK"}
            onClick={() => {
              setQuestion(null);
            }}
          />
          {question.confirmTitle && (
            <Button
              variant="primary"
              title={question.confirmTitle}
              onClick={() => {
                setQuestion(null);
                submit(true);
              }}
            />
          )}
        </AlertDialog>
      )}
    </Dialog>
  );
}

/** The new canvas, dashed, around the result's frame (`ReframePreview`). */
function ReframePreview({ canvas }: { canvas: ReframeCanvas }) {
  return (
    <div
      aria-hidden
      className="relative mx-auto h-160 border-2 border-dashed border-accent"
      style={{ aspectRatio: String(canvas.widthOverHeight ?? 1) }}
    >
      <div
        className="absolute bg-glass-stroke"
        style={{
          left: `${String((canvas.frameX ?? 0) * 100)}%`,
          top: `${String((canvas.frameY ?? 0) * 100)}%`,
          width: `${String((canvas.frameWidth ?? 0) * 100)}%`,
          height: `${String((canvas.frameHeight ?? 0) * 100)}%`,
        }}
      />
    </div>
  );
}

/** Change Voice's voices, each with its kind and a sample to play (`VoicePicker`). */
function VoicePicker({
  selected,
  onPick,
}: {
  selected: string | null;
  onPick: (voice: VoiceRow_Serialize["choice"]) => void;
}) {
  const voices = useQuery({
    queryKey: [...queryKeys.library.all, "voices"],
    queryFn: commands.enhanceVoices,
    retry: false,
  });
  const [playing, setPlaying] = useState<string | null>(null);
  const [loading, setLoading] = useState<string | null>(null);
  const audio = useRef<HTMLAudioElement | null>(null);
  useEffect(() => () => audio.current?.pause(), []);
  const play = (row: VoiceRow_Serialize) => {
    audio.current?.pause();
    if (playing === row.id || row.previewUrl === null) {
      setPlaying(null);
      return;
    }
    const sample = new Audio(row.previewUrl);
    audio.current = sample;
    setLoading(row.id);
    sample.onplaying = () => {
      setLoading(null);
      setPlaying(row.id);
    };
    sample.onended = () => {
      setPlaying(null);
    };
    void sample.play().catch(() => {
      setLoading(null);
    });
  };
  if (voices.isPending) return <Spinner />;
  if (voices.isError) {
    return <p className="m-0 text-caption text-muted">{errorText(voices.error)}</p>;
  }
  return (
    <div className="flex flex-col gap-small">
      <SectionLabel title="Voice" />
      <div role="radiogroup" aria-label="Voice" className="flex max-h-240 flex-col overflow-y-auto">
        {voices.data.map((row) => (
          <div key={row.id} className="flex items-center gap-small">
            <label className="flex flex-1 items-center gap-6 text-body">
              <input
                type="radio"
                name="voice"
                checked={selected === row.choice.id}
                onChange={() => {
                  onPick(row.choice);
                }}
              />
              <span className="flex-1">{row.choice.name}</span>
              <span className="text-caption text-muted">{row.kindLabel}</span>
            </label>
            {row.previewUrl !== null && (
              <Button
                variant="icon"
                icon={playing === row.id ? Pause : Play}
                title={
                  loading === row.id
                    ? "Loading the sample…"
                    : playing === row.id
                      ? "Stop the sample"
                      : "Play a sample"
                }
                hint={
                  loading === row.id
                    ? "Loading the sample…"
                    : playing === row.id
                      ? "Stop the sample"
                      : "Play a sample"
                }
                onClick={() => {
                  play(row);
                }}
              />
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
