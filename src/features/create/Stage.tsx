import { FileDown, Plus, X } from "lucide-react";
import { useEffect, useRef, type KeyboardEvent, type PointerEvent } from "react";

import type { CreateEdit_Deserialize, SourceView_Serialize, TrimPart, TrimView } from "../../bindings";
import { Pill, RenderProgress } from "../../design/Components";
import { cx } from "../../design/cx";
import { Button } from "../../design/Styles";
import { useMediaUrl } from "./createData";

type Edit = (edit: CreateEdit_Deserialize) => void;

/** `CreateStage.emptyStage`: the drop target for a source video. */
export function EmptyStage({
  onPick,
  isLit = false,
}: {
  onPick?: (() => void) | undefined;
  isLit?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onPick}
      data-lit={isLit || undefined}
      className="flex min-h-320 flex-1 cursor-pointer flex-col items-center justify-center gap-10 rounded-card border border-stroke bg-well text-center outline-none focus-visible:border-accent data-lit:border-accent data-lit:bg-accent-wash"
    >
      <FileDown aria-hidden size={36} strokeWidth={1.5} className="text-accent" />
      <span className="font-display text-stage font-heavy text-text uppercase">Drop a video</span>
      <span className="max-w-360 text-muted">
        MOV, MP4, HEVC, ProRes — converted and trimmed for you. The clip needs 4 to 30 seconds of motion.
      </span>
    </button>
  );
}

/** The source playing in the selected range, its pills, and the trim strip under it. */
export function SourceStage({
  source,
  trim,
  edit,
}: {
  source: SourceView_Serialize;
  trim: TrimView | null;
  edit: Edit;
}) {
  const { playback } = source;
  const url = useMediaUrl(playback.path);
  const video = useRef<HTMLVideoElement>(null);
  const range = trim?.ranges[trim.selected];
  const start = range?.start ?? 0;
  const end = range?.end ?? source.duration ?? 0;

  // Plays the selected range on a loop, from its start whenever it moves.
  useEffect(() => {
    const element = video.current;
    if (!element || !url) return;
    if (element.currentTime < start || element.currentTime > end) element.currentTime = start;
    const loop = () => {
      if (element.currentTime >= end) element.currentTime = start;
    };
    element.addEventListener("timeupdate", loop);
    return () => {
      element.removeEventListener("timeupdate", loop);
    };
  }, [url, start, end]);

  return (
    <div className="flex min-w-0 flex-1 flex-col gap-medium">
      <div className="relative flex min-h-320 flex-1 items-center justify-center overflow-hidden rounded-card border border-stroke bg-well">
        {url ? (
          <video
            ref={video}
            src={url}
            aria-label={source.name}
            muted
            autoPlay
            playsInline
            controls
            className="max-h-full max-w-full"
          />
        ) : playback.error !== null ? (
          <p className="m-0 max-w-360 text-center text-danger">{playback.error}</p>
        ) : (
          <div className="flex w-240 flex-col items-center gap-small text-muted">
            <span>Preparing a preview…</span>
            {playback.progress !== null && <RenderProgress value={playback.progress} />}
          </div>
        )}
        <div className="absolute top-12 left-12 flex flex-wrap gap-6">
          {source.pills.map((pill) => (
            <Pill key={pill} text={pill} />
          ))}
        </div>
        <div className="absolute top-12 right-12 flex gap-6">
          <Button
            variant="icon"
            icon={X}
            title="Remove video"
            hint="Remove video"
            onClick={() => {
              edit({ kind: "removeSource" });
            }}
          />
        </div>
      </div>
      <div className="flex items-baseline gap-small text-small">
        <span className="font-semibold text-text">{source.name}</span>
        <span className="text-muted">
          {source.fileText} · {source.sizeText}
        </span>
        <span className="ml-auto text-muted tabular-nums">{source.summary}</span>
      </div>
      {trim && (
        <TrimStrip trim={trim} duration={source.duration ?? 0} thumbnails={source.thumbnails} edit={edit} />
      )}
    </div>
  );
}

function Thumbnail({ path }: { path: string }) {
  const url = useMediaUrl(path);
  return url ? (
    <img src={url} alt="" draggable={false} className="h-full min-w-0 flex-1 object-cover" />
  ) : (
    <span className="h-full flex-1" />
  );
}

/** The step a key moves a handle by: 0.1 s. */
function steps(event: KeyboardEvent): number | null {
  const size = 1;
  if (event.key === "ArrowLeft" || event.key === "ArrowDown") return -size;
  if (event.key === "ArrowRight" || event.key === "ArrowUp") return size;
  return null;
}

/**
 * The trim strip: the clip's frames with every range on them. The selected range has a handle at each end and moves
 * whole when dragged by its middle; the arrow keys nudge the focused part. Rust keeps every range in the model's
 * window and off its neighbors.
 */
function TrimStrip({
  trim,
  duration,
  thumbnails,
  edit,
}: {
  trim: TrimView;
  duration: number;
  thumbnails: string[];
  edit: Edit;
}) {
  const strip = useRef<HTMLDivElement>(null);
  const drag = useRef<{ part: TrimPart; offset: number } | null>(null);
  const percent = (time: number | null) => `${String(duration > 0 ? ((time ?? 0) / duration) * 100 : 0)}%`;

  const timeAt = (clientX: number) => {
    const box = strip.current?.getBoundingClientRect();
    if (!box || box.width === 0) return 0;
    return Math.min(Math.max((clientX - box.left) / box.width, 0), 1) * duration;
  };

  const startDrag = (part: TrimPart, event: PointerEvent<HTMLElement>) => {
    if (trim.isLocked) return;
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    const selected = trim.ranges[trim.selected];
    drag.current = {
      part,
      offset: part === "selection" ? timeAt(event.clientX) - (selected?.start ?? 0) : 0,
    };
  };

  const moveDrag = (event: PointerEvent<HTMLElement>) => {
    const current = drag.current;
    if (!current) return;
    const time = timeAt(event.clientX);
    if (current.part === "start") edit({ kind: "setStart", time });
    else if (current.part === "end") edit({ kind: "setEnd", time });
    else edit({ kind: "moveRange", start: time - current.offset });
  };

  const endDrag = () => {
    drag.current = null;
  };

  const nudge = (part: TrimPart) => (event: KeyboardEvent) => {
    const count = steps(event);
    if (count === null || trim.isLocked) return;
    event.preventDefault();
    edit({ kind: "nudge", part, steps: count });
  };

  const selected = trim.ranges[trim.selected];
  const handleClass =
    "absolute top-0 bottom-0 z-10 w-10 -translate-x-1/2 cursor-ew-resize rounded-small bg-accent outline-none focus-visible:ring-2 focus-visible:ring-text";

  return (
    <div className="flex flex-col gap-small">
      <div
        ref={strip}
        className="relative h-56 overflow-hidden rounded-field border border-stroke bg-well select-none"
        onPointerMove={moveDrag}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
      >
        <div aria-hidden className="flex h-full">
          {thumbnails.map((path) => (
            <Thumbnail key={path} path={path} />
          ))}
        </div>
        {trim.uncovered.map((gap) => (
          <span
            key={`gap-${String(gap.start ?? 0)}`}
            aria-hidden
            className="absolute top-0 bottom-0 bg-scrim"
            style={{ left: percent(gap.start), width: percent((gap.end ?? 0) - (gap.start ?? 0)) }}
          />
        ))}
        {trim.ranges.map((range, index) =>
          index === trim.selected ? null : (
            <button
              key={`range-${String(range.start ?? 0)}`}
              type="button"
              aria-label={`Select range ${range.text}`}
              className="absolute top-0 bottom-0 cursor-pointer border-2 border-glass-stroke bg-transparent"
              style={{ left: percent(range.start), width: percent((range.end ?? 0) - (range.start ?? 0)) }}
              onClick={() => {
                edit({ kind: "selectRange", index });
              }}
            />
          ),
        )}
        {selected && (
          <>
            <div
              role="slider"
              tabIndex={0}
              aria-label="Trim range"
              aria-valuetext={selected.text}
              aria-valuenow={selected.start ?? 0}
              aria-valuemin={0}
              aria-valuemax={duration}
              className="absolute top-0 bottom-0 cursor-grab border-2 border-accent outline-none focus-visible:bg-accent-wash"
              style={{
                left: percent(selected.start),
                width: percent((selected.end ?? 0) - (selected.start ?? 0)),
              }}
              onPointerDown={(event) => {
                startDrag("selection", event);
              }}
              onKeyDown={nudge("selection")}
            />
            <div
              role="slider"
              tabIndex={0}
              aria-label="Trim start"
              aria-valuetext={trim.startValue}
              aria-valuenow={selected.start ?? 0}
              aria-valuemin={0}
              aria-valuemax={duration}
              className={handleClass}
              style={{ left: percent(selected.start) }}
              onPointerDown={(event) => {
                startDrag("start", event);
              }}
              onKeyDown={nudge("start")}
            />
            <div
              role="slider"
              tabIndex={0}
              aria-label="Trim end"
              aria-valuetext={trim.endValue}
              aria-valuenow={selected.end ?? 0}
              aria-valuemin={0}
              aria-valuemax={duration}
              className={handleClass}
              style={{ left: percent(selected.end) }}
              onPointerDown={(event) => {
                startDrag("end", event);
              }}
              onKeyDown={nudge("end")}
            />
          </>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-6 text-small">
        {trim.ranges.length > 1 &&
          trim.ranges.map((range, index) => (
            <span
              key={`chip-${String(range.start ?? 0)}`}
              className={cx(
                "inline-flex h-24 items-center gap-4 rounded-full border px-10 tabular-nums",
                index === trim.selected ? "border-accent text-text" : "border-stroke text-muted",
              )}
            >
              <button
                type="button"
                className="cursor-pointer bg-transparent text-inherit"
                onClick={() => {
                  edit({ kind: "selectRange", index });
                }}
              >
                {range.text}
              </button>
              <button
                type="button"
                aria-label="Remove range"
                className="cursor-pointer bg-transparent text-faint"
                onClick={() => {
                  edit({ kind: "removeRange", index });
                }}
              >
                <X aria-hidden size={11} />
              </button>
            </span>
          ))}
        {!trim.isLocked && (
          <Button
            variant="secondary"
            icon={Plus}
            title="Add range"
            disabled={!trim.canAddRange}
            hint="Add a 4–30 s range"
            onClick={() => {
              edit({ kind: "addRange" });
            }}
          />
        )}
        <span className="ml-auto text-muted tabular-nums">{trim.summary}</span>
      </div>
    </div>
  );
}
