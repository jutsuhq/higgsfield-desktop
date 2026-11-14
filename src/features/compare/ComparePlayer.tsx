import { Pause, Play } from "lucide-react";
import { useEffect, useRef, useState, type PointerEvent } from "react";

import type { CompareLayout } from "../../bindings";
import { MediaTag } from "../../design/Components";
import { DividerLine } from "../../design/DividerLine";
import { Button } from "../../design/Styles";
import { isEditingText } from "../../design/textFocus";
import { compareDuration, dividerAfterKey, dividerAt, hasDrifted, sourceTime, timecode } from "./compareClock";

/** One video of the player: Rust's track, or a batch cell. */
export interface PlayerTrack {
  id: string;
  /** The chips over the video. */
  labels: string[];
  url: string | null;
  /** A source plays the range that was sent, following the result's clock (`CompareTrack.isSource`). */
  source?: { start: number; end: number } | undefined;
}

/**
 * Two or more results on one clock (`ComparePlayer.swift`): Space plays and pauses, the slider scrubs every video,
 * the clock stops at the shortest one and the leftmost result is the one heard. With the divider layout the right
 * video is laid over the left one and revealed right of a divider that can be dragged or moved with ← and →.
 */
export function ComparePlayer({
  tracks,
  layout,
  columns,
  aspect = 16 / 9,
  isActive = true,
}: {
  tracks: PlayerTrack[];
  layout: CompareLayout;
  columns: number;
  /** Width over height of the videos; each pane keeps it. */
  aspect?: number;
  /** False while a sheet is open: Space and the arrows are its own. */
  isActive?: boolean;
}) {
  const videos = useRef<(HTMLVideoElement | null)[]>([]);
  const [durations, setDurations] = useState<number[]>([]);
  const [time, setTime] = useState(0);
  const [isPlaying, setIsPlaying] = useState(false);
  const [divider, setDivider] = useState(0.5);
  // The leftmost result leads the clock and is the one heard; a source follows it from its range's start.
  const lead = Math.max(
    tracks.findIndex((track) => track.source === undefined),
    0,
  );
  const duration = compareDuration(durations.filter((_, index) => tracks[index]?.source === undefined));
  const isDivider = layout === "divider" && tracks.length === 2;

  const each = (run: (video: HTMLVideoElement) => void) => {
    for (const video of videos.current) if (video) run(video);
  };
  /** Where the track at `index` plays at clock time `clock`. */
  const timeOf = (index: number, clock: number) => {
    const range = tracks[index]?.source;
    return range === undefined ? clock : sourceTime(range.start, range.end, clock);
  };
  const seek = (seconds: number) => {
    const clamped = Math.min(Math.max(seconds, 0), duration || seconds);
    videos.current.forEach((video, index) => {
      if (video) video.currentTime = timeOf(index, clamped);
    });
    setTime(clamped);
  };
  const toggle = () => {
    if (isPlaying) {
      each((video) => {
        video.pause();
      });
      setIsPlaying(false);
    } else {
      if (duration > 0 && time >= duration) seek(0);
      each((video) => void video.play().catch(() => undefined));
      setIsPlaying(true);
    }
  };

  // The lead is the clock: the others follow it, and every one stops at the shortest end.
  const onClock = () => {
    const clock = videos.current[lead];
    if (!clock) return;
    const now = clock.currentTime;
    if (duration > 0 && now >= duration) {
      each((video) => {
        video.pause();
      });
      setIsPlaying(false);
      setTime(duration);
      return;
    }
    videos.current.forEach((video, index) => {
      if (index === lead || !video) return;
      const target = timeOf(index, now);
      if (hasDrifted(video.currentTime, target)) video.currentTime = target;
    });
    setTime(now);
  };

  useEffect(() => {
    if (!isActive) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || isEditingText() || event.ctrlKey || event.altKey || event.metaKey) return;
      if (event.key === " ") {
        event.preventDefault();
        toggle();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  });

  const onDrag = (event: PointerEvent<HTMLDivElement>) => {
    if (event.buttons !== 1) return;
    const box = event.currentTarget.getBoundingClientRect();
    setDivider(dividerAt(event.clientX - box.left, box.width));
  };

  const video = (track: PlayerTrack, index: number, className: string) =>
    track.url === null ? (
      <div key={track.id} className="absolute inset-0 flex items-center justify-center text-caption text-faint">
        File not found
      </div>
    ) : (
      <video
        key={track.id}
        ref={(element) => {
          videos.current[index] = element;
        }}
        src={track.url}
        muted={index !== lead}
        playsInline
        preload="metadata"
        onLoadedMetadata={(event) => {
          if (index !== lead) event.currentTarget.currentTime = timeOf(index, time);
          const length = event.currentTarget.duration;
          setDurations((current) => {
            const next = [...current];
            next[index] = length;
            return next;
          });
        }}
        onTimeUpdate={index === lead ? onClock : undefined}
        className={className}
      />
    );

  return (
    <div className="flex flex-col gap-small">
      {isDivider ? (
        <div
          className="relative w-full overflow-hidden rounded-small bg-black"
          style={{ aspectRatio: aspect }}
          onPointerDown={onDrag}
          onPointerMove={onDrag}
        >
          {tracks.map((track, index) => (
            <div
              key={track.id}
              className="absolute inset-0"
              style={index === 1 ? { clipPath: `inset(0 0 0 ${String(divider * 100)}%)` } : undefined}
            >
              {video(track, index, "absolute inset-0 size-full object-contain")}
            </div>
          ))}
          <div
            role="slider"
            tabIndex={0}
            aria-label="Compare divider"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(divider * 100)}
            onKeyDown={(event) => {
              const next = dividerAfterKey(divider, event.key);
              if (next === null) return;
              event.preventDefault();
              event.stopPropagation();
              setDivider(next);
            }}
            className="absolute inset-y-0 w-24 -translate-x-1/2 cursor-ew-resize"
            style={{ left: `${String(divider * 100)}%` }}
          >
            <DividerLine />
          </div>
          <span className="absolute top-small left-small flex gap-x-small">
            {tracks[0]?.labels.map((label) => (
              <MediaTag key={label} text={label} />
            ))}
          </span>
          <span className="absolute top-small right-small flex gap-x-small">
            {tracks[1]?.labels.map((label) => (
              <MediaTag key={label} text={label} />
            ))}
          </span>
        </div>
      ) : (
        <div
          className="grid gap-small"
          style={{ gridTemplateColumns: `repeat(${String(Math.max(columns, 1))}, minmax(0, 1fr))` }}
        >
          {tracks.map((track, index) => (
            <div key={track.id} className="relative overflow-hidden rounded-small bg-black" style={{ aspectRatio: aspect }}>
              {video(track, index, "absolute inset-0 size-full object-contain")}
              <span className="absolute top-small left-small flex flex-wrap gap-x-small">
                {track.labels.map((label) => (
                  <MediaTag key={label} text={label} />
                ))}
              </span>
            </div>
          ))}
        </div>
      )}
      <div className="flex items-center gap-small">
        <Button
          variant="icon"
          icon={isPlaying ? Pause : Play}
          title={isPlaying ? "Pause" : "Play"}
          hint={isPlaying ? "Pause (Space)" : "Play (Space)"}
          onClick={toggle}
        />
        <input
          type="range"
          aria-label="Playback position"
          min={0}
          max={duration || 0}
          step={0.01}
          value={Math.min(time, duration || 0)}
          onChange={(event) => {
            seek(Number(event.currentTarget.value));
          }}
          className="flex-1 accent-accent"
        />
        <span className="text-caption font-medium text-muted tabular-nums">{timecode(time)}</span>
      </div>
    </div>
  );
}
