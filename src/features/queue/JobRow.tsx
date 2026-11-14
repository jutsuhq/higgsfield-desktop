import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import {
  ArrowRight,
  ArrowUpToLine,
  Check,
  CircleArrowDown,
  CopyPlus,
  Folder,
  Hash,
  Pause,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  RotateCw,
  Sparkles,
  Timer,
  TriangleAlert,
  X,
  type LucideIcon,
} from "lucide-react";
import { useLayoutEffect, useRef, useState, type MouseEvent, type ReactNode } from "react";

import {
  commands,
  type JobCommand,
  type JobPreview_Serialize,
  type QueueRow_Serialize,
  type ResultTile_Serialize,
  type RowAction,
  type Stage,
} from "../../bindings";
import { AlertDialog } from "../../app/Dialog";
import { EstimatedProgress, FreshGlow, MediaTag, Pill, RenderProgress } from "../../design/Components";
import { cx } from "../../design/cx";
import { HoverHint } from "../../design/HoverHint";
import { Button, Card } from "../../design/Styles";
import { queryKeys } from "../../lib/queryKeys";
import { loadJob } from "../create/createData";
import { useJobProgress } from "./queueData";

const noteClass = {
  accent: "text-accent",
  info: "text-info",
  warning: "text-warning",
  danger: "text-danger",
  faint: "text-faint",
} as const;

/** What a row's buttons can do beyond its own job commands; all handled by the page. */
export interface RowHandlers {
  perform: (id: string, command: JobCommand) => Promise<void>;
  /** Duplicate job `id`; `undefined` while another duplicate is being queued. */
  duplicate: ((id: string) => void) | undefined;
  /** Hides the job from the Queue; only rows that didn't finish have it. */
  remove?: ((id: string) => void) | undefined;
  selectedId: string | null;
  toggleSelection: (id: string) => void;
}

/** One job in the Queue (`JobRowContent`): preview, title, meta, status, progress and its buttons. */
export function JobRow({ row, handlers }: { row: QueueRow_Serialize; handlers: RowHandlers }) {
  const { model } = row;
  const [isBusy, setIsBusy] = useState(false);
  const progress = useJobProgress(model.id, model.progress);
  const isActive = model.elapsed !== null;
  const isSelected = handlers.selectedId === model.id;

  const perform = (command: JobCommand) => {
    setIsBusy(true);
    void handlers.perform(model.id, command).finally(() => {
      setIsBusy(false);
    });
  };

  return (
    <FreshGlow isFresh={row.isFresh}>
      {/* A click on the row, outside its buttons, selects it for Cancel Job and Retry. */}
      <Card
        isHighlighted={isSelected}
        aria-selected={isSelected}
        role="option"
        aria-label={model.title}
        onClick={() => {
          handlers.toggleSelection(model.id);
        }}
        className="flex cursor-default items-center gap-large p-medium"
      >
        <JobPreview preview={row.preview} height={isActive ? 116 : 64} />
        <div className="flex min-w-0 flex-1 flex-col gap-5">
          <p className={cx("m-0 truncate font-semibold text-text", isActive ? "text-button" : "text-body")}>
            {model.title}
          </p>
          <HoverHint text={`${model.priceSource}: ${model.cost}`}>
            <span className="truncate text-small font-medium text-faint tabular-nums">{model.meta}</span>
          </HoverHint>
          {model.origin !== null && (
            <HoverHint text={model.originDetail ?? model.origin}>
              <span className="truncate text-small font-medium text-muted">{model.origin}</span>
            </HoverHint>
          )}
          {isActive && model.prompt !== null && (
            <HoverHint text={row.promptHint ?? model.prompt}>
              <span className="truncate text-small text-muted">{model.prompt}</span>
            </HoverHint>
          )}
          {model.note !== null ? (
            <p className={cx("m-0 line-clamp-2 text-small font-medium", noteClass[model.note.role])}>
              {model.note.text}
            </p>
          ) : (
            model.stage !== null && (
              <>
                <StageStepper
                  stage={model.stage}
                  detail={[model.progressText, model.generating].filter((part) => part !== null).join(" · ")}
                />
                {progress !== null ? (
                  <div className="max-w-420 pt-2">
                    <RenderProgress value={progress} />
                  </div>
                ) : (
                  model.timeEstimate !== null && (
                    <div className="max-w-420 pt-2">
                      <EstimatedProgress
                        estimate={{ ...model.timeEstimate, fraction: model.timeEstimate.fraction ?? 0 }}
                      />
                    </div>
                  )
                )}
              </>
            )
          )}
        </div>
        <div className="flex shrink-0 flex-col items-end gap-small">
          {model.statusHint !== null ? (
            <HoverHint text={model.statusHint}>
              <Pill text={model.statusLabel} dot={model.statusRole} isPulsing={model.isWorking} />
            </HoverHint>
          ) : (
            <Pill text={model.statusLabel} dot={model.statusRole} isPulsing={model.isWorking} />
          )}
          {model.elapsed !== null && (
            <HoverHint text="Time since queued or retried">
              <span className="inline-flex items-center gap-4 text-caption-plus font-medium text-faint tabular-nums">
                <Timer aria-hidden size={11} />
                {model.elapsed}
              </span>
            </HoverHint>
          )}
          {/* The buttons don't select the row. */}
          <div
            className="flex items-center gap-6"
            onClick={(event: MouseEvent) => {
              event.stopPropagation();
            }}
          >
            {model.actions.map((action) => (
              <ActionButton key={action.kind} action={action} row={row} isBusy={isBusy} perform={perform} />
            ))}
            {row.supportIds !== null && <CopyIds ids={row.supportIds} disabled={isBusy} />}
            <Button
              variant="secondary"
              isLabelHidden
              icon={CopyPlus}
              title="Duplicate"
              hint="Duplicate job"
              disabled={isBusy || handlers.duplicate === undefined}
              onClick={() => handlers.duplicate?.(model.id)}
            />
            {handlers.remove && (
              <Button
                variant="secondary"
                isLabelHidden
                icon={X}
                title="Remove"
                hint="Remove from Queue"
                disabled={isBusy}
                onClick={() => handlers.remove?.(model.id)}
              />
            )}
          </div>
        </div>
      </Card>
    </FreshGlow>
  );
}

function CopyIds({ ids, disabled }: { ids: string; disabled: boolean }) {
  return (
    <Button
      variant="secondary"
      icon={Hash}
      title="Copy IDs"
      hint="Copy job IDs"
      disabled={disabled}
      onClick={() => {
        void navigator.clipboard.writeText(ids);
      }}
    />
  );
}

function ActionButton({
  action,
  row,
  isBusy,
  perform,
}: {
  action: RowAction;
  row: QueueRow_Serialize;
  isBusy: boolean;
  perform: (command: JobCommand) => void;
}) {
  const navigate = useNavigate();
  // Edit and Generate again load the job's inputs into Create, then open it.
  const openCreate = () => {
    void loadJob(row.model.id).then(() => navigate({ to: "/create" }));
  };
  switch (action.kind) {
    case "cancel":
      return (
        <Button
          variant="secondary"
          title="Cancel"
          hint={action.help}
          disabled={isBusy || !action.isEnabled}
          onClick={() => {
            perform("cancel");
          }}
        />
      );
    case "retry":
      return (
        <Button
          variant="secondary"
          icon={RotateCw}
          title="Retry"
          hint="Retry"
          disabled={isBusy}
          onClick={() => {
            perform("retry");
          }}
        />
      );
    case "retryDownload":
      return (
        <Button
          variant="secondary"
          icon={CircleArrowDown}
          title="Retry download"
          hint="Retry download"
          disabled={isBusy}
          onClick={() => {
            perform("retry");
          }}
        />
      );
    case "checkAgain":
      return (
        <Button
          variant="secondary"
          icon={RotateCw}
          title="Check again"
          hint="Check again"
          disabled={isBusy}
          onClick={() => {
            perform("checkAgain");
          }}
        />
      );
    case "generateAgain":
      return (
        <Button
          variant="secondary"
          icon={RefreshCw}
          title="Generate again"
          hint="Open in Create"
          disabled={isBusy}
          onClick={openCreate}
        />
      );
    case "resume":
      return (
        <Button
          variant="secondary"
          icon={Play}
          isIconFilled
          title="Resume"
          hint="Resume"
          disabled={isBusy}
          onClick={() => {
            perform("resume");
          }}
        />
      );
    case "runNext":
      return (
        <Button
          variant="secondary"
          icon={ArrowUpToLine}
          title="Run next"
          hint="Run next"
          disabled={isBusy}
          onClick={() => {
            perform("runNext");
          }}
        />
      );
    case "confirmPrice":
      return (
        <ConfirmPriceButton
          row={row}
          disabled={isBusy}
          resume={() => {
            perform("resume");
          }}
        />
      );
    case "edit":
      return (
        <Button
          variant="secondary"
          icon={Pencil}
          title="Edit"
          hint="Open in Create"
          disabled={isBusy}
          onClick={openCreate}
        />
      );
    case "showInFolder":
      return (
        <Button
          variant="secondary"
          isLabelHidden
          icon={Folder}
          title="Show in File Manager"
          hint="Show in File Manager"
          disabled={isBusy}
          onClick={() => {
            void commands.queueShowInFolder(row.model.id);
          }}
        />
      );
    case "openInLibrary":
      return (
        <Button
          variant="secondary"
          isLabelHidden
          icon={ArrowRight}
          title="Open in Library"
          hint="Open in Library"
          disabled={isBusy}
          onClick={() => {
            void navigate({ to: "/library/result/$id", params: { id: row.model.id } });
          }}
        />
      );
  }
}

/** Resumes a job paused for its server price, after the user agrees to that price (`ConfirmPriceButton`). */
function ConfirmPriceButton({
  row,
  disabled,
  resume,
}: {
  row: QueueRow_Serialize;
  disabled: boolean;
  resume: () => void;
}) {
  const [isConfirming, setIsConfirming] = useState(false);
  const confirmation = row.priceConfirmation;
  return (
    <>
      <Button
        variant="secondary"
        icon={Play}
        isIconFilled
        title="Review price"
        hint="Price above estimate"
        disabled={disabled || confirmation === null}
        onClick={() => {
          setIsConfirming(true);
        }}
      />
      {isConfirming && confirmation !== null && (
        <AlertDialog
          title={confirmation.title}
          message={confirmation.message}
          onCancel={() => {
            setIsConfirming(false);
          }}
        >
          <Button
            variant="secondary"
            title="Not now"
            onClick={() => {
              setIsConfirming(false);
            }}
          />
          <Button
            variant="primary"
            title={confirmation.confirmTitle}
            onClick={() => {
              setIsConfirming(false);
              resume();
            }}
          />
        </AlertDialog>
      )}
    </>
  );
}

const stages: { stage: Stage; title: string }[] = [
  { stage: "prepare", title: "Prepare" },
  { stage: "upload", title: "Upload" },
  { stage: "generate", title: "Generate" },
  { stage: "save", title: "Save" },
];

/**
 * Prepare → Upload → Generate → Save, with the current step lit and its detail beside it (`StageStepper`): a check on
 * the steps done, a ring around the current one. Labels on every step when they fit, otherwise only on the current one.
 */
function StageStepper({ stage, detail }: { stage: Stage; detail: string }) {
  const current = stages.findIndex((step) => step.stage === stage);
  const row = useRef<HTMLDivElement>(null);
  const full = useRef<HTMLDivElement>(null);
  const [labelsAll, setLabelsAll] = useState(true);
  // `ViewThatFits`: every label when the row with all of them (laid out unseen) fits, else the current one alone.
  useLayoutEffect(() => {
    const fit = () => {
      if (row.current && full.current) setLabelsAll(full.current.offsetWidth <= row.current.clientWidth);
    };
    fit();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(fit);
    if (row.current) observer?.observe(row.current);
    return () => {
      observer?.disconnect();
    };
  }, [stage, detail]);
  return (
    <div
      ref={row}
      role="img"
      aria-label={`Step ${String(current + 1)} of 4, ${stages[current]?.title ?? ""}`}
      className="relative pt-6"
    >
      <div ref={full} aria-hidden className="invisible absolute top-6 left-0 flex w-max items-center gap-6">
        <Steps current={current} detail={detail} labelsAll />
      </div>
      <div className="flex items-center gap-6 overflow-hidden">
        <Steps current={current} detail={detail} labelsAll={labelsAll} />
      </div>
    </div>
  );
}

function Steps({ current, detail, labelsAll }: { current: number; detail: string; labelsAll: boolean }) {
  return (
    <>
      {stages.map((step, index) => (
        <span key={step.stage} aria-hidden className="inline-flex shrink-0 items-center gap-6">
          {index > 0 && (
            <span className={cx("h-2 w-16 rounded-full", index <= current ? "bg-accent/60" : "bg-stroke")} />
          )}
          <span className="inline-flex items-center gap-5">
            {index < current ? (
              <span className="flex size-14 items-center justify-center rounded-full bg-accent text-ink">
                <Check size={7} strokeWidth={5} />
              </span>
            ) : index === current ? (
              <span className="flex size-14 items-center justify-center rounded-full border-[1.5px] border-accent">
                <span className="size-6 rounded-full bg-accent" />
              </span>
            ) : (
              <span className="size-14 rounded-full border-[1.5px] border-stroke" />
            )}
            {(labelsAll || index === current) && (
              <span
                className={cx(
                  "text-caption-plus whitespace-nowrap tabular-nums",
                  index === current ? "font-semibold text-text" : "font-medium",
                  index < current ? "text-muted" : index > current ? "text-faint" : undefined,
                )}
              >
                {index === current && detail !== "" ? `${step.title} ${detail}` : step.title}
              </span>
            )}
          </span>
        </span>
      ))}
    </>
  );
}

const videoExtension = /\.(mp4|mov|m4v|webm)$/i;

/** Tags only on tiles wide enough to hold them; narrow portrait tiles read fine without. */
const tagWidth = 76;

function useMediaUrl(path: string | null): string | null | undefined {
  return useQuery({
    queryKey: [...queryKeys.queue.all, "media", path],
    queryFn: () => (path === null ? null : commands.mediaUrl(path)),
    enabled: path !== null,
  }).data;
}

/**
 * What goes in and what comes out of a job (`JobPreview`): a frame of the source, "+", the reference images fanned
 * out, an arrow, then the result. A clip made without a source shows no source.
 */
function JobPreview({ preview, height }: { preview: JobPreview_Serialize; height: number }) {
  const width = Math.round(height * (preview.aspect ?? 16 / 9));
  const showsTags = width >= tagWidth;
  return (
    <div className="flex shrink-0 items-center gap-8">
      {preview.hasSource && (
        <FrameTile path={preview.source} width={width} height={height} tag={showsTags ? "Source" : null} />
      )}
      {preview.references.length > 0 && (
        <>
          {preview.hasSource && <Connector icon={Plus} />}
          <ReferenceStack
            references={preview.references}
            more={preview.moreReferences}
            size={Math.round(height * 0.56)}
          />
        </>
      )}
      <Connector icon={ArrowRight} />
      <ResultTile tile={preview.result} width={width} height={height} showsTag={showsTags} />
    </div>
  );
}

function Connector({ icon: Icon }: { icon: LucideIcon }) {
  return <Icon aria-hidden size={10} strokeWidth={3.5} className="shrink-0 text-faint" />;
}

/** A frame of a video or an image, served by `media_url`; a plain well until it loads. */
function FrameTile({
  path,
  width,
  height,
  tag = null,
  children,
}: {
  path: string | null;
  width: number;
  height: number;
  tag?: string | null;
  children?: ReactNode;
}) {
  const url = useMediaUrl(path);
  return (
    <div
      className="relative shrink-0 overflow-hidden rounded-tile border border-stroke bg-well"
      style={{ width, height }}
    >
      {url && path !== null && videoExtension.test(path) && (
        <video aria-hidden src={url} muted preload="metadata" className="size-full object-cover" />
      )}
      {url && path !== null && !videoExtension.test(path) && (
        <img src={url} alt="" className="size-full object-cover" />
      )}
      {tag !== null && (
        <span className="absolute top-5 left-5">
          <MediaTag text={tag} />
        </span>
      )}
      {children}
    </div>
  );
}

/** Up to three reference images fanned out, with a count when there are more. */
function ReferenceStack({
  references,
  more,
  size,
}: {
  references: (string | null)[];
  more: number;
  size: number;
}) {
  const step = size * 0.32;
  return (
    <div
      className="relative shrink-0"
      style={{ width: size + Math.max(references.length - 1, 0) * step, height: size }}
    >
      {references.map((path, index) => (
        <ReferenceThumb
          key={index}
          path={path}
          size={size}
          left={index * step}
          zIndex={references.length - index}
        />
      ))}
      {more > 0 && (
        <span className="absolute -right-4 -bottom-4 z-10">
          <MediaTag text={`+${String(more)}`} />
        </span>
      )}
    </div>
  );
}

function ReferenceThumb({
  path,
  size,
  left,
  zIndex,
}: {
  path: string | null;
  size: number;
  left: number;
  zIndex: number;
}) {
  const url = useMediaUrl(path);
  return (
    <span
      className="absolute top-0 overflow-hidden rounded-small border-2 border-background bg-well"
      style={{ width: size, height: size, left, zIndex }}
    >
      {url && <img src={url} alt="" className="size-full object-cover" />}
    </span>
  );
}

/**
 * The output (`ResultTile`): a frame once it is saved, a moving signature shimmer while it renders, or an icon for a
 * job that stopped.
 */
function ResultTile({
  tile,
  width,
  height,
  showsTag,
}: {
  tile: ResultTile_Serialize;
  width: number;
  height: number;
  showsTag: boolean;
}) {
  switch (tile.kind) {
    case "done":
      return (
        <FrameTile path={tile.path} width={width} height={height} tag={showsTag ? "Result" : null}>
          <span className="absolute right-5 bottom-5 flex size-18 items-center justify-center rounded-full bg-accent text-ink">
            <Check size={9} strokeWidth={4} />
          </span>
        </FrameTile>
      );
    case "paused":
      return <Placeholder icon={Pause} isFilled tint="text-warning" width={width} height={height} />;
    case "failed":
      return <Placeholder icon={TriangleAlert} tint="text-danger" width={width} height={height} />;
    case "canceled":
      return <Placeholder icon={X} tint="text-faint" width={width} height={height} />;
    case "rendering":
      return <RenderingTile width={width} height={height} />;
  }
}

function Placeholder({
  icon: Icon,
  isFilled = false,
  tint,
  width,
  height,
}: {
  icon: LucideIcon;
  isFilled?: boolean;
  tint: string;
  width: number;
  height: number;
}) {
  return (
    <div
      className="flex shrink-0 items-center justify-center rounded-tile border border-stroke bg-well"
      style={{ width, height }}
    >
      <Icon size={14} strokeWidth={2.5} fill={isFilled ? "currentColor" : "none"} className={tint} />
    </div>
  );
}

/**
 * The signature gradient with a light band sweeping across: the app doing its trick (`RenderingTile`). With Reduce
 * Motion the band rests off the tile and the sparkles hold still (`motion.css`).
 */
function RenderingTile({ width, height }: { width: number; height: number }) {
  return (
    <div
      role="img"
      aria-label="Rendering"
      className="relative shrink-0 overflow-hidden rounded-tile"
      style={{ width, height }}
    >
      <span className="absolute inset-0 bg-(image:--signature-diagonal) opacity-55" />
      {/* At rest (Reduce Motion) the band sits one tile width left of the tile, out of sight. */}
      <span
        className="absolute inset-y-0 left-0 w-[60%] animate-render-sweep bg-(image:--sweep-band)"
        style={{ transform: "translateX(calc(-100% / 0.6))" }}
      />
      <span className="absolute inset-0 flex items-center justify-center text-text">
        <Sparkles size={16} strokeWidth={2.5} className="animate-pulse-dot" />
      </span>
    </div>
  );
}
