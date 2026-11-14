import { Check, Cloud, HardDrive, Star, X, type LucideIcon } from "lucide-react";
import { useEffect, useRef, useState, type MouseEvent } from "react";

import type { LibraryCard_Serialize } from "../../bindings";
import { MediaTag } from "../../design/Components";
import { cx } from "../../design/cx";
import { Spinner } from "../../design/Spinner";
import { focusRing } from "../../design/Styles";
import { useThumbnail } from "./libraryData";

/** The card's frame: the media server's file, Rust's cached frame, or the account's link for a result only there. */
export function CardMedia({ card, isHovered }: { card: LibraryCard_Serialize; isHovered: boolean }) {
  const isVideo = card.kind === "video";
  const isLocal = card.filePath !== null && !card.isUnavailable;
  const frame = useThumbnail(card.id, isVideo && isLocal).data ?? null;
  const fits = card.fit === "fitOverBackdrop";
  const media = "absolute inset-0 size-full";
  // The frame pulses until its picture is decoded, then the picture fades in: no black tile, no pop.
  const [isLoaded, setIsLoaded] = useState(false);
  if (card.isUnavailable || card.mediaUrl === null) {
    return (
      <div className="absolute inset-0 flex flex-col items-center justify-center gap-6 bg-well text-caption text-faint">
        {card.isUnavailable && card.kind === "image" && <HardDrive aria-hidden size={20} />}
        {card.isUnavailable ? "Can't be found" : "No preview"}
      </div>
    );
  }
  const fit = fits ? "object-contain" : "object-cover";
  const shown = cx(media, fit, isLoaded ? "animate-fade-in" : "invisible");
  const onLoaded = () => {
    setIsLoaded(true);
  };
  const still = isVideo ? (frame ?? undefined) : card.mediaUrl;
  const backdrop = fits && still !== undefined && isLoaded;
  return (
    <>
      {!isLoaded && <span aria-hidden className={cx(media, "animate-pulse bg-glass")} />}
      {backdrop && <img aria-hidden src={still} alt="" className={cx(media, "scale-110 object-cover blur-xl")} />}
      {isVideo && (isHovered || still === undefined) ? (
        <HoverVideo src={card.mediaUrl} poster={still} isPlaying={isHovered} className={shown} onLoaded={onLoaded} />
      ) : (
        <img src={still} alt="" className={shown} onLoad={onLoaded} />
      )}
    </>
  );
}

/**
 * The card's video: it plays only while the pointer is over the card and goes back to its first frame after. With
 * no still to show, the first frame is the picture, so the file opens at a moment past zero to have one decoded.
 */
function HoverVideo({
  src,
  poster,
  isPlaying,
  className,
  onLoaded,
}: {
  src: string;
  poster: string | undefined;
  isPlaying: boolean;
  className: string;
  onLoaded: () => void;
}) {
  const video = useRef<HTMLVideoElement>(null);
  useEffect(() => {
    const element = video.current;
    if (!element) return;
    if (isPlaying) {
      // A play the browser refuses or a pause interrupts leaves the still frame, which is what the card shows anyway.
      void element.play().catch(() => undefined);
    } else {
      element.pause();
      if (element.readyState > 0) element.currentTime = firstFrame;
    }
  }, [isPlaying]);
  return (
    <video
      ref={video}
      src={poster === undefined ? `${src}#t=${String(firstFrame)}` : src}
      poster={poster}
      muted
      loop
      playsInline
      preload={poster === undefined ? "metadata" : "auto"}
      onLoadedData={onLoaded}
      className={className}
    />
  );
}

/** Where a card's video rests, in seconds: just past zero, so a frame is decoded and shown. */
const firstFrame = 0.05;

export interface CardHandlers {
  onActivate: (card: LibraryCard_Serialize, event: MouseEvent) => void;
  onMenu: (card: LibraryCard_Serialize, event: MouseEvent) => void;
  onDoubleClick?: (card: LibraryCard_Serialize) => void;
}

/** A run on this computer making the item's next version, or its subtitles: how far it is (`OnDeviceRunBadge`). */
export interface CardRun {
  progress: string;
  fraction: number | null;
}

/**
 * One card of the grid (`LibraryCards.swift`, `ImageCardFrame`, `LibraryCardMarks`): the 16:9 tile that plays on
 * hover, its tags and badges, the run on it, the verdict and variant marks, and one line of title and one of detail.
 * Hovered, selected or picked, the tile's border turns lime; the card the arrow keys are on has the same ring.
 */
export function LibraryCard({
  card,
  isSelected,
  isPicked = false,
  isCurrent,
  run = null,
  handlers,
}: {
  card: LibraryCard_Serialize;
  isSelected: boolean;
  /** Picked to compare. */
  isPicked?: boolean;
  isCurrent: boolean;
  run?: CardRun | null;
  handlers: CardHandlers;
}) {
  const [isHovered, setIsHovered] = useState(false);
  const isImage = card.kind === "image";
  const isLit = isHovered || isPicked || isSelected;
  const hint = isImage
    ? card.isUnavailable
      ? "File missing or drive disconnected"
      : card.prompt === ""
        ? undefined
        : card.prompt
    : undefined;
  return (
    <div
      role="gridcell"
      aria-selected={isSelected || isPicked}
      aria-current={isCurrent || undefined}
      data-card={card.id}
      className={cx("group relative flex h-full flex-col gap-2", card.culling === "rejected" && "opacity-45")}
      onMouseEnter={() => {
        setIsHovered(true);
      }}
      onMouseLeave={() => {
        setIsHovered(false);
      }}
      onContextMenu={(event) => {
        event.preventDefault();
        handlers.onMenu(card, event);
      }}
    >
      <div className="relative mb-7">
        {card.batch !== null && (
          // The edge of a second card behind, so a batch reads as a stack.
          <span aria-hidden className="absolute inset-x-10 -top-5 h-full rounded-media bg-card" />
        )}
        <button
          type="button"
          aria-label={card.accessibilityLabel}
          title={hint}
          onClick={(event) => {
            handlers.onActivate(card, event);
          }}
          onDoubleClick={() => handlers.onDoubleClick?.(card)}
          className={cx(
            "relative block aspect-video w-full cursor-pointer overflow-hidden rounded-media bg-card",
            focusRing,
          )}
        >
          <CardMedia card={card} isHovered={isHovered} />
          <span className="absolute top-8 left-8 flex gap-4">
            <MediaTag text={card.modeTag} />
            {card.batch !== null ? (
              <MediaTag text={card.batch.tag} />
            ) : (
              !isImage && card.isDraft && <MediaTag text="Draft" />
            )}
          </span>
          <span className="absolute top-8 right-8 flex gap-4">
            {card.isStarred && <Badge icon={Star} label={card.batch === null ? "Starred" : "Has starred results"} />}
            {card.needsImport && card.batch === null && <Badge icon={Cloud} label="On Higgsfield, not downloaded yet" />}
          </span>
          <span className="absolute inset-x-0 top-8 flex justify-center gap-4">
            {card.variantCount !== null && <MediaTag text={`${String(card.variantCount)} variants`} />}
            {card.isVariantPrimary && card.variantCount === null && card.variantGroup !== null && (
              <MediaTag text="Primary" />
            )}
            {card.culling === "kept" && <Badge icon={Check} label="Kept" />}
            {card.culling === "rejected" && <Badge icon={X} label="Rejected" isDanger />}
          </span>
          {card.batch === null && (card.durationText ?? card.aspectText) !== null && (
            <span className="absolute right-8 bottom-8">
              <MediaTag text={card.durationText ?? card.aspectText ?? ""} />
            </span>
          )}
          {(card.isDownloading || run !== null) && (
            <span className="absolute inset-0 flex items-center justify-center">
              {card.isDownloading ? <Spinner label="Downloading" /> : run && <RunBadge run={run} />}
            </span>
          )}
          {(isPicked || isSelected) && (
            <span className="absolute bottom-8 left-8">
              <SelectionCheck label={isPicked ? "Picked to compare" : "Selected"} />
            </span>
          )}
          <span
            aria-hidden
            className={cx(
              "pointer-events-none absolute inset-0 rounded-media border transition-colors duration-120",
              isLit || isCurrent ? "border-2 border-accent" : "border-stroke",
            )}
          />
        </button>
      </div>
      <span className="truncate text-body font-semibold text-text">{card.title}</span>
      <span className="truncate text-small font-medium text-faint">{card.subtitle}</span>
    </div>
  );
}

/** A starred, not-downloaded or verdict badge: a small icon on a dark circle. */
function Badge({ icon: Icon, label, isDanger = false }: { icon: LucideIcon; label: string; isDanger?: boolean }) {
  return (
    <span
      role="img"
      aria-label={label}
      className={cx(
        "flex size-17 items-center justify-center rounded-full bg-scrim",
        isDanger ? "text-danger" : "text-accent",
      )}
    >
      <Icon aria-hidden size={9} strokeWidth={4} fill={Icon === Star ? "currentColor" : "none"} />
    </span>
  );
}

/** The lime check on a card that is picked to compare or selected. */
function SelectionCheck({ label }: { label: string }) {
  return (
    <span role="img" aria-label={label} className="flex size-20 items-center justify-center rounded-full bg-accent text-ink">
      <Check aria-hidden size={10} strokeWidth={4} />
    </span>
  );
}

/** How far a run on the card is: its bar, or a spinner while it waits, over its text. */
export function RunBadge({ run }: { run: CardRun }) {
  return (
    <span
      role="status"
      aria-label={run.progress}
      className="flex flex-col items-center gap-6 rounded-small bg-scrim px-10 py-8"
    >
      {run.fraction === null ? (
        <Spinner />
      ) : (
        <span className="h-4 w-120 overflow-hidden rounded-full bg-track">
          <span className="block h-full rounded-full bg-accent" style={{ width: `${String(run.fraction * 100)}%` }} />
        </span>
      )}
      <span className="truncate text-caption font-semibold text-text">{run.progress}</span>
    </span>
  );
}
