import { useQueryClient } from "@tanstack/react-query";
import {
  ArrowLeftToLine,
  ArrowRightToLine,
  AudioLines,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  CloudDownload,
  Film,
  Image as ImageIcon,
  Info,
  Plus,
  RefreshCw,
  TriangleAlert,
  Video,
  WandSparkles,
  X,
  XCircle,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useId, useRef, useState, type ChangeEvent, type KeyboardEvent } from "react";

import {
  commands,
  type CinemaView_Serialize,
  type Choice_Serialize,
  type CreateEdit_Deserialize,
  type CreateSnapshot_Serialize,
  type FrameSlot,
  type FramesView_Serialize,
  type ImageView_Serialize,
  type LengthView,
  type LinkDownloadView_Serialize,
  type PickerView_Serialize,
  type PromptView,
  type SpeechView,
  type ReferencesView_Serialize,
  type ToggleView_Serialize,
  type VariantsView_Serialize,
} from "../../bindings";
import { useToastStore } from "../../app/toastStore";
import { DropZone, MediaTag, SectionLabel } from "../../design/Components";
import { cx } from "../../design/cx";
import { HoverHint } from "../../design/HoverHint";
import { FilterMenu, SegmentedControl } from "../../design/Pickers";
import { Button, focusRing } from "../../design/Styles";
import { ValueStepper } from "../../design/ValueStepper";
import { queryKeys } from "../../lib/queryKeys";
import { CharacterSets } from "./CharacterSets";
import { GenerateReferenceSheet, useReferencePicks } from "./GenerateReferences";
import { HiggsfieldReferences } from "./HiggsfieldReferences";
import { LinkDownloadRow, PasteLinkButton } from "./Links";
import { chooseSource, showError, useCreateNotice, useMediaUrl } from "./createData";
import { PresetSection } from "./PresetSection";
import { PromptClearButton, PromptHistoryButton, replacePrompt } from "./PromptHistory";
import { SubmitBar } from "./SubmitBar";

type Edit = (edit: CreateEdit_Deserialize) => void;

const selectClass = cx(
  "h-34 w-full rounded-field border border-stroke bg-well px-small text-body text-text",
  "disabled:opacity-40 enabled:cursor-pointer",
  focusRing,
);

/** Mode tabs and tasks: a choice with a reason can't be picked, and says why instead. */
function pickChoice(choices: Choice_Serialize[], value: string, pick: (value: string) => void) {
  const reason = choices.find((choice) => choice.value === value)?.unavailableReason;
  if (reason) useCreateNotice.getState().show({ title: "Not available", detail: reason });
  else pick(value);
}

/** A disabled group of controls, dimmed as SwiftUI's `.disabled` dims them. */
const disabledGroup = "m-0 flex min-w-0 flex-col gap-small border-0 p-0 disabled:opacity-40";

/**
 * The video side's panel (`CreatePanel`): the form scrolls, with Variants and the cost line and buttons pinned under
 * it. Top to bottom: mode, the model with its task and options, Batch, the source, the clip's length and frame or
 * what Extend adds, the speech, the styles, the characters or the frames and references, the prompt, the resolution,
 * the audio and the draft length.
 */
export function CreatePanel({ snapshot, edit }: { snapshot: CreateSnapshot_Serialize; edit: Edit }) {
  const client = useQueryClient();
  /** What the last change of model, tab or task did to the images' roles (`CreateForm.recheckingRoles`). */
  const [roleChange, setRoleChange] = useState<string | null>(null);
  /** Applies a change of model, tab or task and says what it did to the images' roles; Undo brings back what it cut. */
  const recheckRoles: Edit = (change) => {
    commands.createChangeRoles(change).then(
      (outcome) => {
        client.setQueryData(queryKeys.create.snapshot, outcome.snapshot);
        if (outcome.canUndo)
          useToastStore.getState().showDraftReplaced(() => void commands.createUndoReplacement());
        setRoleChange(outcome.note);
      },
      (error: unknown) => {
        showError("Can't change the form", error);
      },
    );
  };
  return (
    <>
      <div className="flex min-h-0 flex-1 flex-col gap-18 overflow-y-auto p-large">
        <SegmentedControl
          aria-label="Mode"
          value={snapshot.tab}
          options={snapshot.tabs}
          onChange={(mode) => {
            pickChoice(snapshot.tabs, mode, () => {
              recheckRoles({ kind: "setTab", mode });
            });
          }}
        />
        {roleChange !== null && (
          <RoleChangeNote
            text={roleChange}
            onDismiss={() => {
              setRoleChange(null);
            }}
          />
        )}
        {snapshot.models.length > 1 && (
          <ModelSection snapshot={snapshot} edit={edit} recheckRoles={recheckRoles} />
        )}
        {snapshot.batch && (
          <label className="flex cursor-pointer items-center gap-small rounded-field border border-stroke bg-well p-medium">
            <span className="flex flex-1 flex-col gap-2">
              <span className="text-body font-semibold text-text">Batch</span>
              <span className="text-caption text-faint">{snapshot.batch.hint}</span>
            </span>
            <input
              type="checkbox"
              role="switch"
              aria-label="Batch"
              className="accent-accent"
              checked={snapshot.batch.isOn}
              onChange={(event) => {
                edit({ kind: "setBatch", on: event.target.checked });
              }}
            />
          </label>
        )}
        <SourceSection snapshot={snapshot} edit={edit} />
        {(snapshot.clipLength || snapshot.aspectRatio) && (
          <section className="flex flex-col gap-small">
            {snapshot.clipLength && <LengthSlider length={snapshot.clipLength} edit={edit} />}
            {snapshot.aspectRatio && <FrameMenu picker={snapshot.aspectRatio} edit={edit} />}
          </section>
        )}
        {snapshot.addedLength && (
          <section className="flex flex-col gap-small">
            <LengthSlider length={snapshot.addedLength} edit={edit} />
          </section>
        )}
        {snapshot.speech && <Speech speech={snapshot.speech} edit={edit} />}
        {snapshot.presets && (
          <PresetSection
            presets={snapshot.presets}
            onPick={(id) => {
              edit({ kind: "pickPreset", id });
            }}
          />
        )}
        {snapshot.characterSets ? (
          <CharacterSets sets={snapshot.characterSets} edit={edit} />
        ) : (
          <>
            {snapshot.frames && <Frames frames={snapshot.frames} edit={edit} />}
            <References
              references={snapshot.references}
              offersHiggsfield={snapshot.offersHiggsfieldReferences}
              linkDownload={snapshot.linkDownload}
              edit={edit}
            />
          </>
        )}
        <Prompt prompt={snapshot.prompt} edit={edit} />
        <div className="flex flex-col gap-small">
          <Picker
            picker={snapshot.klingQuality}
            onPick={(quality) => {
              edit({ kind: "setKlingQuality", quality });
            }}
          />
          <Picker
            picker={snapshot.resolution}
            onPick={(resolution) => {
              edit({ kind: "setResolution", resolution });
            }}
          />
          {snapshot.bitrate && (
            <div className="flex pt-small">
              <HintedGroup
                picker={snapshot.bitrate}
                isLabel
                onPick={(bitrate) => {
                  edit({ kind: "setBitrate", bitrate });
                }}
              />
            </div>
          )}
        </div>
        <Picker
          picker={snapshot.audio}
          onPick={(audio) => {
            edit({ kind: "setAudio", audio });
          }}
        />
        <Picker
          picker={snapshot.draftLength}
          onPick={(length) => {
            edit({
              kind: "setDraftLength",
              length: length === "firstFiveSeconds" ? "firstFiveSeconds" : "wholeRange",
            });
          }}
        />
      </div>
      <div aria-hidden className="h-1 shrink-0 bg-stroke" />
      {/* Next to the buttons rather than in the scrolled form: it multiplies what they cost. */}
      <div className="flex shrink-0 flex-col gap-medium p-large">
        {snapshot.variants && <Variants variants={snapshot.variants} edit={edit} />}
        <SubmitBar view={snapshot.submit} />
      </div>
    </>
  );
}

/** What a change of model did to the images' roles, under the mode and model pickers until dismissed. */
function RoleChangeNote({ text, onDismiss }: { text: string; onDismiss: () => void }) {
  return (
    <div
      role="status"
      className="flex items-start gap-small rounded-field border border-stroke bg-well p-medium"
    >
      <Info aria-hidden size={14} className="shrink-0 text-accent" />
      <p className="m-0 min-w-0 flex-1 text-caption text-muted">{text}</p>
      <button
        type="button"
        aria-label="Dismiss"
        className={cx("flex shrink-0 cursor-pointer text-faint", focusRing)}
        onClick={onDismiss}
      >
        <X aria-hidden size={12} strokeWidth={2.5} />
      </button>
    </div>
  );
}

/**
 * Create's model picker (`ModelPicker`): a capsule pull-down with the selected model, opening one flat list where each
 * model shows its price per second and, when the models differ in it, its longest length.
 */
function ModelPicker({
  rows,
  selected,
  onPick,
}: {
  rows: CreateSnapshot_Serialize["models"];
  selected: string;
  onPick: (id: string) => void;
}) {
  const [isOpen, setIsOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const title = rows.find((row) => row.id === selected)?.title ?? selected;
  const valueId = useId();

  useEffect(() => {
    if (!isOpen) return;
    const checked = menu.current?.querySelector<HTMLElement>("[aria-checked=true]");
    (checked ?? menu.current?.querySelector<HTMLElement>("[role=menuitemradio]:not(:disabled)"))?.focus();
    const close = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setIsOpen(false);
    };
    document.addEventListener("pointerdown", close);
    return () => {
      document.removeEventListener("pointerdown", close);
    };
  }, [isOpen]);

  const closeMenu = () => {
    setIsOpen(false);
    button.current?.focus();
  };

  const onMenuKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const items = [
      ...(menu.current?.querySelectorAll<HTMLElement>("[role=menuitemradio]:not(:disabled)") ?? []),
    ];
    const index = items.indexOf(document.activeElement as HTMLElement);
    if (event.key === "Escape" || event.key === "Tab") {
      event.preventDefault();
      closeMenu();
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      items[Math.min(index + 1, items.length - 1)]?.focus();
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      items[Math.max(index - 1, 0)]?.focus();
    }
  };

  return (
    <div ref={root} className="relative flex">
      <button
        ref={button}
        type="button"
        aria-label="Model"
        aria-haspopup="menu"
        aria-expanded={isOpen}
        aria-describedby={valueId}
        onClick={() => {
          setIsOpen((open) => !open);
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown") {
            event.preventDefault();
            setIsOpen(true);
          }
        }}
        className={cx(
          "flex min-h-34 w-full cursor-pointer items-center gap-6 rounded-full border border-stroke bg-well px-14",
          "text-muted transition-colors duration-120 ease-out hovered:bg-glass-hover hovered:text-text",
          focusRing,
        )}
      >
        <span id={valueId} className="min-w-0 flex-1 truncate text-left text-control font-semibold">
          {title}
        </span>
        <ChevronDown aria-hidden size={9} strokeWidth={3.5} className="shrink-0 text-faint" />
      </button>
      {isOpen && (
        <div
          ref={menu}
          role="menu"
          aria-label="Model"
          onKeyDown={onMenuKey}
          className="absolute top-full left-0 z-40 mt-x-small flex min-w-full flex-col rounded-field border border-glass-stroke bg-background p-x-small shadow-toast"
        >
          {rows.map((row) => {
            const isChecked = row.id === selected;
            const label = row.detail === null ? row.title : `${row.title} · ${row.detail}`;
            return (
              <button
                key={row.id}
                type="button"
                role="menuitemradio"
                aria-checked={isChecked}
                disabled={row.unavailableReason !== null}
                title={row.unavailableReason ?? undefined}
                onClick={() => {
                  onPick(row.id);
                  closeMenu();
                }}
                className="flex h-24 cursor-pointer items-center gap-6 rounded-small px-small text-left text-body whitespace-nowrap text-text outline-none hover:bg-selection focus:bg-selection disabled:cursor-default disabled:text-faint disabled:hover:bg-transparent"
              >
                <Check aria-hidden size={12} strokeWidth={2.5} className={isChecked ? "" : "invisible"} />
                {label}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

/** Only where more than one model is offered: the model, its task, Kling's options and Cinema's direction. */
function ModelSection({
  snapshot,
  edit,
  recheckRoles,
}: {
  snapshot: CreateSnapshot_Serialize;
  edit: Edit;
  /** For a change of model or task, which can move the images between roles. */
  recheckRoles: Edit;
}) {
  const selectedModel = snapshot.models.find((row) => row.id === snapshot.model);
  return (
    <Section title="Model" detail={selectedModel?.detail ?? undefined}>
      <ModelPicker
        rows={snapshot.models}
        selected={snapshot.model}
        onPick={(model) => {
          recheckRoles({ kind: "setModel", model });
        }}
      />
      {snapshot.tasks.length >= 2 && (
        <SegmentedControl
          aria-label="Task"
          value={snapshot.mode}
          options={snapshot.tasks}
          onChange={(mode) => {
            pickChoice(snapshot.tasks, mode, () => {
              recheckRoles({ kind: "pickTask", mode });
            });
          }}
        />
      )}
      {(snapshot.videoBackground || snapshot.orientation) && (
        <div className="flex flex-col gap-small">
          {snapshot.videoBackground && <BackgroundToggle toggle={snapshot.videoBackground} edit={edit} />}
          {snapshot.orientation && (
            <HintedGroup
              picker={snapshot.orientation}
              onPick={(orientation) => {
                edit({ kind: "setOrientation", orientation });
              }}
            />
          )}
        </div>
      )}
      {snapshot.cinema && <CinemaOptions cinema={snapshot.cinema} edit={edit} />}
    </Section>
  );
}

function Section({
  title,
  detail,
  children,
}: {
  title: string;
  detail?: string | undefined;
  children: React.ReactNode;
}) {
  return (
    <section className="flex flex-col gap-small">
      <SectionLabel title={title} detail={detail} />
      {children}
    </section>
  );
}

/**
 * A model control: segments when every choice can be picked and they fit, else a menu. Disabled with its reason
 * when the connection can't send it.
 */
function Picker({
  picker,
  onPick,
}: {
  picker: PickerView_Serialize | null;
  onPick: (value: string) => void;
}) {
  if (!picker) return null;
  const isDisabled = picker.unavailableReason !== null;
  const asSegments =
    !isDisabled && picker.options.length <= 4 && picker.options.every((option) => !option.unavailableReason);
  const control = asSegments ? (
    <SegmentedControl
      aria-label={picker.title}
      value={picker.selected}
      options={picker.options}
      onChange={onPick}
    />
  ) : (
    <select
      aria-label={picker.title}
      className={selectClass}
      value={picker.selected}
      disabled={isDisabled}
      onChange={(event: ChangeEvent<HTMLSelectElement>) => {
        onPick(event.target.value);
      }}
    >
      {picker.options.map((option) => (
        <option key={option.value} value={option.value} disabled={option.unavailableReason !== null}>
          {option.unavailableReason ? `${option.title} — ${option.unavailableReason}` : option.title}
        </option>
      ))}
    </select>
  );
  return (
    <Section title={picker.title} detail={picker.detail ?? undefined}>
      {picker.hint ? (
        <HoverHint text={picker.hint}>
          <span className="flex">{control}</span>
        </HoverHint>
      ) : (
        control
      )}
      {isDisabled && <p className="m-0 text-small text-faint">{picker.unavailableReason}</p>}
    </Section>
  );
}

/**
 * Seedance's bitrate and Kling's "Character faces" (`BitratePicker`, `KlingOptions`): a title over segments,
 * disabled where the connection can't send it, with what it does, or why not, as the hint.
 */
function HintedGroup({
  picker,
  isLabel = false,
  onPick,
}: {
  picker: PickerView_Serialize;
  /** Titled with a section label (the bitrate) rather than a control caption (Kling). */
  isLabel?: boolean;
  onPick: (value: string) => void;
}) {
  return (
    <HoverHint text={picker.hint ?? undefined} className="flex w-full">
      <fieldset disabled={picker.unavailableReason !== null} className={cx(disabledGroup, "w-full")}>
        {isLabel ? (
          <SectionLabel title={picker.title} />
        ) : (
          <span className="text-control font-medium text-muted">{picker.title}</span>
        )}
        <SegmentedControl
          aria-label={picker.title}
          value={picker.selected}
          options={picker.options}
          onChange={onPick}
        />
      </fieldset>
    </HoverHint>
  );
}

/** "Keep the video's background": off as well as disabled where the connection can't keep it. */
function BackgroundToggle({ toggle, edit }: { toggle: ToggleView_Serialize; edit: Edit }) {
  const isDisabled = toggle.unavailableReason !== null;
  return (
    <HoverHint text={toggle.hint ?? undefined} className="flex w-full">
      <label className="flex w-full cursor-pointer items-center gap-small has-disabled:cursor-default has-disabled:opacity-40">
        <span className="flex-1 text-control font-medium text-muted">{toggle.title}</span>
        <input
          type="checkbox"
          role="switch"
          aria-label={toggle.title}
          className="accent-accent"
          checked={toggle.isOn && !isDisabled}
          disabled={isDisabled}
          onChange={(event) => {
            edit({ kind: "setKeepsVideoBackground", on: event.target.checked });
          }}
        />
      </label>
    </HoverHint>
  );
}

/**
 * Cinema Studio's creative controls (`CinemaOptions`): one closed Direction row with "Auto" or "2 set", which opens to
 * a menu per control, each on Auto (the director picks it) until set.
 */
function CinemaOptions({ cinema, edit }: { cinema: CinemaView_Serialize; edit: Edit }) {
  const [isOpen, setIsOpen] = useState(false);
  return (
    <HoverHint text={cinema.hint} className="flex w-full">
      <div className="flex w-full flex-col gap-small">
        <button
          type="button"
          aria-expanded={isOpen}
          aria-label={`Direction, ${cinema.summary}`}
          className={cx("flex min-h-28 cursor-pointer items-center gap-small text-left", focusRing)}
          onClick={() => {
            setIsOpen((open) => !open);
          }}
        >
          <span className="flex-1 text-control font-medium text-muted">Direction</span>
          <span className={cx("text-small font-semibold", cinema.isSet ? "text-accent" : "text-faint")}>
            {cinema.summary}
          </span>
          <ChevronRight
            aria-hidden
            size={10}
            strokeWidth={3}
            className={cx("text-faint transition-transform duration-150 ease-out", isOpen && "rotate-90")}
          />
        </button>
        {isOpen && (
          <fieldset
            disabled={cinema.unavailableReason !== null}
            className="m-0 min-w-0 rounded-field border border-stroke bg-well px-medium py-0 disabled:opacity-40"
          >
            {cinema.controls.map((control, index) => (
              <div
                key={control.title}
                className={cx("flex min-h-40 items-center gap-small", index > 0 && "border-t border-stroke")}
              >
                <span className="min-w-0 flex-1 truncate text-body text-text">{control.title}</span>
                <FilterMenu
                  title={control.title}
                  value={control.selected}
                  options={control.options}
                  onChange={(value) => {
                    edit({
                      kind: "setCinema",
                      control: control.detail ?? control.title,
                      value: value === "" ? null : value,
                    });
                  }}
                />
              </div>
            ))}
          </fieldset>
        )}
      </div>
    </HoverHint>
  );
}

/** Length, or what Extend adds: a slider in whole seconds over the model's window, its value beside the title. */
function LengthSlider({ length, edit }: { length: LengthView; edit: Edit }) {
  return (
    <>
      <SectionLabel title={length.title} detail={length.detail} />
      <input
        type="range"
        aria-label={length.title}
        aria-valuetext={length.detail}
        min={length.min ?? undefined}
        max={length.max ?? undefined}
        step={1}
        value={length.value ?? length.min ?? 0}
        className="w-full accent-accent"
        onChange={(event) => {
          edit({ kind: "setLength", seconds: Math.round(Number(event.target.value)) });
        }}
      />
    </>
  );
}

/** The frame of a clip made without a source: seven frames don't fit a segmented row, so a menu. */
function FrameMenu({ picker, edit }: { picker: PickerView_Serialize; edit: Edit }) {
  return (
    <div className="flex items-center gap-small pt-small">
      <div className="flex-1">
        <SectionLabel title={picker.title} />
      </div>
      <FilterMenu
        title={picker.title}
        value={picker.selected}
        options={picker.options}
        onChange={(ratio) => {
          edit({ kind: "setAspectRatio", ratio });
        }}
      />
    </div>
  );
}

/** Several results of one submit; held at 1 where the jobs make one, with the reason under it. */
function Variants({ variants, edit }: { variants: VariantsView_Serialize; edit: Edit }) {
  const reason = variants.unavailableReason;
  return (
    <div className="flex flex-col gap-x-small">
      <div className="flex items-center gap-small">
        <div className="flex-1">
          <SectionLabel title="Variants" detail={reason === null ? "billed each" : undefined} />
        </div>
        <HoverHint text="Number of variants">
          <fieldset disabled={reason !== null} className={disabledGroup}>
            <ValueStepper
              label="Variants"
              value={variants.value}
              min={variants.min}
              max={variants.max}
              onChange={(count) => {
                edit({ kind: "setVariants", count });
              }}
            />
          </fieldset>
        </HoverHint>
      </div>
      {reason !== null && <p className="m-0 text-caption text-faint">{reason}</p>}
    </div>
  );
}

/** An empty, dashed drop zone that opens a file picker (`DropButton`); files can also be dropped on the window. */
function DropButton({
  title,
  detail,
  icon: Icon,
  onClick,
}: {
  title: string;
  detail: string;
  icon: LucideIcon;
  onClick: () => void;
}) {
  return (
    <DropZone className="flex">
      <button
        type="button"
        className={cx(
          "relative flex w-full cursor-pointer flex-col items-center gap-6 rounded-field py-20",
          focusRing,
        )}
        onClick={onClick}
      >
        <span className="mb-2 flex size-36 items-center justify-center rounded-full bg-accent/10 text-accent">
          <Icon aria-hidden size={16} strokeWidth={2.5} />
        </span>
        <span className="text-body font-semibold text-text">{title}</span>
        <span className="text-caption-plus text-faint">{detail}</span>
      </button>
    </DropZone>
  );
}

/** The source video (`SourceSection`): its name and the selected ranges, the upscale notice, or Add a video. */
function SourceSection({ snapshot, edit }: { snapshot: CreateSnapshot_Serialize; edit: Edit }) {
  return (
    <section className="flex flex-col gap-small">
      <div className="flex items-center gap-small">
        <div className="flex-1">
          <SectionLabel title="Source video" detail={snapshot.sourceDetail} />
        </div>
        <PasteLinkButton kind="video" isDisabled={snapshot.linkDownload !== null} />
      </div>
      {snapshot.linkDownload?.section === "video" && <LinkDownloadRow download={snapshot.linkDownload} />}
      {snapshot.source ? (
        <>
          <div className="flex items-center gap-10 rounded-field border border-stroke bg-well p-medium">
            <Film aria-hidden size={17} className="shrink-0 text-muted" />
            <span className="flex min-w-0 flex-1 flex-col gap-2">
              <span className="truncate text-body text-text">{snapshot.source.name}</span>
              <span className="text-caption text-faint tabular-nums">{snapshot.source.summary}</span>
            </span>
            <HoverHint text="Remove video">
              <button
                type="button"
                aria-label="Remove video"
                className={cx("flex cursor-pointer text-accent", focusRing)}
                onClick={() => {
                  edit({ kind: "removeSource" });
                }}
              >
                <XCircle aria-hidden size={16} />
              </button>
            </HoverHint>
          </div>
          {snapshot.upscale && (
            <div className="flex items-start gap-small rounded-field border border-stroke bg-well p-medium">
              <TriangleAlert aria-hidden size={16} className="shrink-0 text-warning" />
              <span className="flex min-w-0 flex-1 flex-col gap-2">
                <span className="text-small font-bold text-text">{snapshot.upscale.title}</span>
                <span className="text-caption text-muted">{snapshot.upscale.detail}</span>
              </span>
              <input
                type="checkbox"
                role="switch"
                aria-label="Upscale before upload"
                className="accent-accent"
                checked={snapshot.upscale.allows}
                onChange={(event) => {
                  edit({ kind: "setAllowsUpscale", allows: event.target.checked });
                }}
              />
            </div>
          )}
        </>
      ) : (
        <DropButton title="Add a video" detail={snapshot.sourceHint} icon={Video} onClick={chooseSource} />
      )}
    </section>
  );
}

/**
 * The prompt, typed here and sent as it changes. The text on screen is the field's own while it has the focus, so
 * an answer for an earlier keystroke never moves the caret.
 */
function Prompt({ prompt, edit }: { prompt: PromptView; edit: Edit }) {
  const id = useId();
  const [text, setText] = useState(prompt.text);
  const field = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    if (document.activeElement !== field.current) setText(prompt.text);
  }, [prompt.text]);
  return (
    <section className="flex flex-col gap-small">
      <div className="flex items-center gap-small text-faint">
        <label htmlFor={id} className="mr-auto text-label font-bold tracking-label uppercase">
          {prompt.title}
        </label>
        <span className="text-caption font-semibold tabular-nums">{prompt.counter}</span>
        {/* Through `replacePrompt`, so the toast offers the prompt back with Undo. */}
        {text !== "" && <PromptClearButton clear={() => void replacePrompt("")} />}
        <PromptHistoryButton />
      </div>
      {/* Grows with the text from 72 to 140 px (`.frame(minHeight: 72, maxHeight: 140)`), then scrolls. */}
      <textarea
        id={id}
        ref={field}
        rows={3}
        value={text}
        placeholder={prompt.hint}
        className={cx(
          "field-sizing-content max-h-140 min-h-72 w-full resize-none rounded-field border border-stroke bg-well p-small",
          "text-body text-text outline-none placeholder:text-faint focus:border-accent",
        )}
        onChange={(event) => {
          setText(event.target.value);
          edit({ kind: "setPrompt", text: event.target.value });
        }}
      />
    </section>
  );
}

function ImageThumbnail({ image, className }: { image: ImageView_Serialize; className?: string }) {
  const url = useMediaUrl(image.thumbnail);
  return url ? (
    <img src={url} alt="" draggable={false} className={cx("object-cover", className)} />
  ) : (
    <span className={cx("bg-glass", className)} />
  );
}

/** A small lime icon button on a scrim, for actions drawn over an image (`TileButton`); `hint` is also its name. */
function TileButton({
  icon: Icon,
  hint,
  disabled = false,
  onClick,
}: {
  icon: LucideIcon;
  hint: string;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <HoverHint text={hint}>
      <button
        type="button"
        aria-label={hint}
        disabled={disabled}
        className={cx(
          "m-3 flex size-18 cursor-pointer items-center justify-center rounded-full bg-scrim text-accent",
          "disabled:cursor-default disabled:text-faint",
          focusRing,
        )}
        onClick={onClick}
      >
        <Icon aria-hidden size={9} strokeWidth={4} />
      </button>
    </HoverHint>
  );
}

function chooseFrame(slot: FrameSlot) {
  commands.createChooseFrame(slot).catch((error: unknown) => {
    showError("Can't add the frame", error);
  });
}

/**
 * The start and end frames (`FramesSection`): optional slots above the references (the start alone on a model without
 * an end frame), each filled from the file picker. Disabled, with the reason as the hint, where the connection can't
 * send frames.
 */
function Frames({ frames, edit }: { frames: FramesView_Serialize; edit: Edit }) {
  const isDisabled = frames.unavailableReason !== null;
  return (
    <HoverHint text={frames.hint} className="flex w-full">
      <fieldset disabled={isDisabled} className={cx(disabledGroup, "w-full")}>
        <SectionLabel title="Frames" detail="Optional" />
        <div className="flex gap-small">
          {frames.slots.map((frame) =>
            frame.image ? (
              <div key={frame.slot} className="relative h-72 flex-1 overflow-hidden rounded-small bg-well">
                <ImageThumbnail image={frame.image} className="size-full" />
                <span className="absolute top-0 right-0">
                  <TileButton
                    icon={X}
                    hint="Remove"
                    onClick={() => {
                      edit({ kind: "clearFrame", slot: frame.slot });
                    }}
                  />
                </span>
                <span className="absolute bottom-3 left-3">
                  <MediaTag text={frame.title} />
                </span>
              </div>
            ) : (
              <button
                key={frame.slot}
                type="button"
                aria-label={frame.title}
                className={cx(
                  "flex min-h-72 flex-1 cursor-pointer flex-col items-center justify-center gap-4 rounded-small",
                  "border border-stroke bg-well disabled:cursor-default",
                  focusRing,
                )}
                onClick={() => {
                  chooseFrame(frame.slot);
                }}
              >
                {frame.slot === "start" ? (
                  <ArrowRightToLine aria-hidden size={14} className="text-accent" />
                ) : (
                  <ArrowLeftToLine aria-hidden size={14} className="text-accent" />
                )}
                <span className="text-caption text-muted">{frame.title}</span>
              </button>
            ),
          )}
        </div>
      </fieldset>
    </HoverHint>
  );
}

function chooseReferences(show: (notice: string) => void) {
  commands.createChooseReferences().then(
    (outcome) => {
      if (outcome.notice !== null) show(outcome.notice);
    },
    (error: unknown) => {
      showError("Can't add the images", error);
    },
  );
}

/**
 * References (`ReferencesSection`): numbered tiles in prompt order, four to a row, each removable and moved left or
 * right. Off while Seedance starts from frames, with the reason under it.
 */
function References({
  references,
  offersHiggsfield,
  linkDownload,
  edit,
}: {
  references: ReferencesView_Serialize;
  offersHiggsfield: boolean;
  linkDownload: LinkDownloadView_Serialize | null;
  edit: Edit;
}) {
  const [isBrowsing, setIsBrowsing] = useState(false);
  const [isGenerating, setIsGenerating] = useState(false);
  const offersGenerate = useReferencePicks().data?.offered ?? false;
  const [notice, setNotice] = useState<string | null>(null);
  const reason = references.disabledReason;
  const pick = () => {
    setNotice(null);
    chooseReferences(setNotice);
  };
  const count = references.items.length;
  return (
    <section className="flex flex-col gap-small">
      <fieldset
        disabled={reason !== null}
        className="m-0 flex min-w-0 flex-col gap-small border-0 p-0 disabled:opacity-50"
      >
        <div className="flex items-center gap-small">
          <div className="flex-1">
            <SectionLabel title={references.title} detail={references.countText} />
          </div>
          {offersHiggsfield && references.canAdd && (
            <Button
              variant="icon"
              icon={CloudDownload}
              title="From Higgsfield"
              hint="Pick from Higgsfield uploads"
              className="size-26"
              onClick={() => {
                setIsBrowsing(true);
              }}
            />
          )}
          {references.canAdd && <PasteLinkButton kind="image" isDisabled={linkDownload !== null} />}
          {offersGenerate && references.canAdd && (
            <Button
              variant="icon"
              icon={WandSparkles}
              title="Generate…"
              hint="Generate reference images"
              className="size-26"
              onClick={() => {
                setIsGenerating(true);
              }}
            />
          )}
        </div>
        {linkDownload?.section === "image" && <LinkDownloadRow download={linkDownload} />}
        {count === 0 ? (
          <DropButton title="Add images" detail={references.hint} icon={ImageIcon} onClick={pick} />
        ) : (
          <ul className="m-0 grid list-none grid-cols-4 gap-small p-0">
            {references.items.map((image, index) => (
              <li
                key={image.path}
                aria-label={`Image ${String(index + 1)}: ${image.name}`}
                className="relative h-72 overflow-hidden rounded-small bg-well"
              >
                <ImageThumbnail image={image} className="size-full" />
                <span className="absolute top-3 left-3">
                  <HoverHint text={`Image ${String(index + 1)} in the prompt`}>
                    <MediaTag text={String(index + 1)} />
                  </HoverHint>
                </span>
                <span className="absolute top-0 right-0">
                  <TileButton
                    icon={X}
                    hint="Remove"
                    onClick={() => {
                      edit({ kind: "removeReference", index });
                    }}
                  />
                </span>
                <span className="absolute inset-x-0 bottom-0 flex justify-between">
                  <TileButton
                    icon={ChevronLeft}
                    hint="Move left"
                    disabled={index === 0}
                    onClick={() => {
                      edit({ kind: "moveReference", from: index, to: index - 1 });
                    }}
                  />
                  <TileButton
                    icon={ChevronRight}
                    hint="Move right"
                    disabled={index === count - 1}
                    onClick={() => {
                      edit({ kind: "moveReference", from: index, to: index + 1 });
                    }}
                  />
                </span>
              </li>
            ))}
            {references.canAdd && (
              <li>
                <HoverHint text="Add images" className="flex">
                  <button
                    type="button"
                    aria-label="Add images"
                    className={cx(
                      "flex min-h-72 w-full cursor-pointer items-center justify-center rounded-small border border-stroke bg-well text-accent",
                      focusRing,
                    )}
                    onClick={pick}
                  >
                    <Plus aria-hidden size={17} />
                  </button>
                </HoverHint>
              </li>
            )}
          </ul>
        )}
      </fieldset>
      {reason !== null && <p className="m-0 text-caption text-faint">{reason}</p>}
      {notice !== null && <p className="m-0 text-small text-warning">{notice}</p>}
      {isBrowsing && (
        <HiggsfieldReferences
          onClose={() => {
            setIsBrowsing(false);
          }}
        />
      )}
      {isGenerating && (
        <GenerateReferenceSheet
          onClose={() => {
            setIsGenerating(false);
          }}
        />
      )}
    </section>
  );
}

/** Where files dropped on the window become the speech (`CreatePage`'s drop routing). */
export const speechDropTarget = "speech";

function chooseSpeech() {
  commands.createChooseSpeech().then(
    (notice) => {
      if (notice !== null) useCreateNotice.getState().show(notice);
    },
    (error: unknown) => {
      showError("Can't add the recording", error);
    },
  );
}

/** Speech (`SpeechSection.swift`): the recording the characters say, by name and length; picked or dropped. */
function Speech({ speech, edit }: { speech: SpeechView; edit: Edit }) {
  return (
    <section className="flex flex-col gap-small" data-drop={speechDropTarget}>
      <SectionLabel title="Speech" detail={speech.detail ?? undefined} />
      {speech.name !== null ? (
        <div className="flex items-center gap-small rounded-small border border-stroke bg-well p-small">
          <AudioLines aria-hidden size={16} className="text-accent" />
          <span className="min-w-0 flex-1 truncate text-body text-text">{speech.name}</span>
          <TileButton icon={RefreshCw} hint="Replace" onClick={chooseSpeech} />
          <TileButton
            icon={X}
            hint="Remove"
            onClick={() => {
              edit({ kind: "removeSpeech" });
            }}
          />
        </div>
      ) : (
        <DropButton
          title="Add speech"
          detail="A voice recording the characters say"
          icon={AudioLines}
          onClick={chooseSpeech}
        />
      )}
    </section>
  );
}
