import { useQuery } from "@tanstack/react-query";
import { Video } from "lucide-react";
import { useEffect, useState } from "react";

import {
  commands,
  events,
  type AppError,
  type MotionClipView_Serialize,
  type MotionLibraryView_Serialize,
} from "../../bindings";
import { MediaTag } from "../../design/Components";
import { cx } from "../../design/cx";
import { HoverHint } from "../../design/HoverHint";
import { LoopingVideo } from "../../design/LoopingVideo";
import { PagedFeedFooter } from "../../design/PagedFeedFooter";
import { SegmentedControl } from "../../design/Pickers";
import { RemoteThumbnail } from "../../design/RemoteThumbnail";
import { Spinner } from "../../design/Spinner";
import { Button, focusRing } from "../../design/Styles";
import { useInvalidateOn } from "../../lib/events";
import { queryKeys } from "../../lib/queryKeys";
import { showError } from "./createData";

/** The connection's Motion Library; `null` without one. `CreateChanged` asks again as pages arrive. */
export function useMotionLibrary(isEnabled: boolean) {
  useInvalidateOn(events.createChanged, [queryKeys.create.motion]);
  return useQuery<MotionLibraryView_Serialize | null, AppError>({
    queryKey: queryKeys.create.motion,
    queryFn: () => commands.createMotionLibrary(),
    enabled: isEnabled,
  });
}

function load(shelf: string | null, more: boolean) {
  commands.createLoadMotionLibrary(shelf, more).catch((error: unknown) => {
    showError("Can't load the library", error);
  });
}

/**
 * The empty stage on an account (`MotionLibraryBrowser.swift`): Higgsfield's Motion Library to start from, or the
 * user's own video. Hovering a clip plays its example; picking one downloads the clip and makes it the source.
 */
export function MotionLibraryBrowser({
  library,
  onPickFile,
}: {
  library: MotionLibraryView_Serialize;
  onPickFile: () => void;
}) {
  const { shelf } = library;
  // Each shelf loads its first page when it is first shown.
  useEffect(() => {
    load(shelf, false);
  }, [shelf]);

  const isBusy = library.downloadingId !== null;
  return (
    <div className="flex min-h-320 min-w-320 flex-1 flex-col gap-large rounded-card border border-stroke bg-well p-large">
      <div className="flex flex-wrap items-start justify-between gap-medium">
        <div className="flex flex-col gap-4">
          <h2 className="m-0 font-display text-stage font-heavy text-text uppercase">Start from a clip</h2>
          <p className="m-0 text-muted">Hover to preview a motion, click to use it.</p>
        </div>
        <Button variant="secondary" icon={Video} title="Choose Video…" onClick={onPickFile} />
      </div>
      <div className="max-w-full overflow-x-auto [scrollbar-width:none]">
        <SegmentedControl
          aria-label="Clips"
          isCompact
          value={library.shelf}
          options={library.shelves}
          onChange={(value) => {
            load(value, false);
          }}
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <ul className="m-0 grid list-none grid-cols-[repeat(auto-fill,minmax(120px,1fr))] gap-medium p-0">
          {library.showsSkeleton &&
            Array.from({ length: 10 }, (_, index) => (
              <li key={`skeleton-${String(index)}`} aria-hidden>
                <span className="block aspect-9/16 animate-pulse rounded-field bg-glass" />
              </li>
            ))}
          {library.items.map((clip) => (
            <li key={clip.id}>
              <ClipTile clip={clip} isDownloading={library.downloadingId === clip.id} isDisabled={isBusy} />
            </li>
          ))}
        </ul>
        <div className="pb-large">
          <PagedFeedFooter
            failure={library.failure ?? undefined}
            hasMore={library.hasMore}
            count={library.items.length}
            hidesSpinner={library.items.length === 0}
            loadMore={() => {
              load(null, true);
            }}
          />
        </div>
      </div>
    </div>
  );
}

/** A portrait thumbnail that plays the example on hover. */
function ClipTile({
  clip,
  isDownloading,
  isDisabled,
}: {
  clip: MotionClipView_Serialize;
  isDownloading: boolean;
  isDisabled: boolean;
}) {
  const [isHovering, setIsHovering] = useState(false);
  return (
    <HoverHint text="Use as source" className="block w-full">
      <button
        type="button"
        disabled={isDisabled}
        aria-label={`${clip.name}, use as source`}
        className={cx(
          "relative block aspect-9/16 w-full overflow-hidden rounded-field border bg-well enabled:cursor-pointer",
          isHovering ? "border-2 border-accent" : "border-stroke",
          focusRing,
        )}
        onMouseEnter={() => {
          setIsHovering(true);
        }}
        onMouseLeave={() => {
          setIsHovering(false);
        }}
        onClick={() => {
          commands.createUseMotionClip(clip.id).catch((error: unknown) => {
            showError("Can't download the clip", error);
          });
        }}
      >
        {/* Pulses until the picture, which covers it, has loaded. */}
        <span aria-hidden className="absolute inset-0 animate-pulse bg-glass" />
        <RemoteThumbnail url={clip.thumbnailUrl} className="absolute inset-0 size-full" />
        {isHovering && clip.previewUrl !== null && (
          <span className="absolute inset-0 motion-reduce:hidden">
            <LoopingVideo src={clip.previewUrl} />
          </span>
        )}
        <span className="absolute bottom-6 left-6 flex flex-col items-start gap-3">
          {clip.example !== null && <MediaTag text={clip.example} />}
          <MediaTag text={clip.name} />
        </span>
        {isDownloading && (
          <span className="absolute inset-0 flex items-center justify-center bg-scrim">
            <Spinner label="Downloading" />
          </span>
        )}
      </button>
    </HoverHint>
  );
}
