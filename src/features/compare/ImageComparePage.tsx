import { useQuery } from "@tanstack/react-query";
import { useNavigate, useParams } from "@tanstack/react-router";
import { ArrowLeftRight, ChevronDown, ChevronLeft, ChevronUp, CircleX, Maximize2, ZoomIn } from "lucide-react";
import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";

import {
  commands,
  type CompareImageView_Serialize as CompareImageView,
  type ImageCompareSides_Serialize as ImageCompareSides,
  type ImageCompareView_Serialize as ImageCompareView,
  type SidesChange,
} from "../../bindings";
import { EmptyState } from "../../design/Components";
import { Spinner } from "../../design/Spinner";
import { Button } from "../../design/Styles";
import { isEditingText } from "../../design/textFocus";
import { queryKeys } from "../../lib/queryKeys";
import { useLibraryView } from "../library/libraryData";
import {
  actualSizeState,
  displayScale,
  fitScale,
  fitState,
  isActualSize,
  isFit,
  maxZoom,
  minZoom,
  panned,
  withZoom,
  type ImageCompareState,
} from "./imageCompareState";

/** The two images picked in the Library, side by side at the same zoom (`ImageCompareView(pair:)`). */
export function ImageComparePage() {
  const ids = (useParams({ strict: false }).ids ?? "").split(",").filter((id) => id !== "");
  const view = useQuery({
    queryKey: [...queryKeys.library.all, "imageCompare", ids],
    queryFn: () => commands.libraryImageCompare(ids),
  });
  return <ImageCompareScreen view={view.data} isError={view.isError} />;
}

/** Compare with Original on a saved image: the version it was made from on the left (`MaterialVersions.imagePair`). */
export function ImageOriginalPage() {
  const id = useParams({ strict: false }).id ?? "";
  const view = useQuery({
    queryKey: [...queryKeys.library.all, "imageCompareOriginal", id],
    queryFn: () => commands.libraryImageCompareOriginal(id),
  });
  return <ImageCompareScreen view={view.data} isError={view.isError} />;
}

/** Compare Variants on an image: the group as the grid shows it, two at a time (`ImageCompareView(variants:)`). */
export function ImageVariantsPage() {
  const group = useParams({ strict: false }).group ?? "";
  const { origin, kind, filter, expandedGroups } = useLibraryView();
  const request = { origin, kind, filter, expandedGroups };
  const view = useQuery({
    queryKey: [...queryKeys.library.all, "imageVariants", group, request],
    queryFn: () => commands.libraryImageVariants(request, group),
  });
  return <ImageCompareScreen view={view.data} isError={view.isError} />;
}

/**
 * Two images side by side at the same zoom: one state drives both panes, so they stay in step. Esc goes back, ← and →
 * swap the sides, and in a variant group ↑ and ↓ step the right side through the others.
 */
function ImageCompareScreen({ view, isError }: { view: ImageCompareView | undefined; isError: boolean }) {
  const navigate = useNavigate();
  const [state, setState] = useState<ImageCompareState>(fitState);
  const [changed, setChanged] = useState<ImageCompareSides | null>(null);
  const back = () => {
    window.history.back();
  };
  const sides = changed !== null && view && fits(changed, view.images.length) ? changed : (view?.sides ?? null);
  const change = (sidesChange: SidesChange) => {
    if (!view || sides === null) return;
    void commands.libraryImageCompareSides(sides, view.images.length, sidesChange).then(setChanged);
  };
  const pages = sides?.position != null;

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || isEditingText() || event.ctrlKey || event.altKey || event.metaKey) return;
      if (event.key === "Escape") back();
      else if (event.key === "ArrowLeft" || event.key === "ArrowRight") change("swap");
      else if (event.key === "ArrowUp" && pages) change("previous");
      else if (event.key === "ArrowDown" && pages) change("next");
      else return;
      event.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  });

  const left = sides === null ? undefined : view?.images[sides.left];
  const right = sides === null ? undefined : view?.images[sides.right];
  const topBar = (
    <div className="flex items-center gap-small">
      <Button variant="secondary" icon={ChevronLeft} title="Back" hint="Back (Esc)" onClick={back} />
      <h1 className="m-0 mr-auto pl-small text-icon font-bold text-text">Compare</h1>
      {sides?.position != null && (
        <div className="flex items-center gap-x-small pr-medium">
          <Button
            variant="icon"
            icon={ChevronUp}
            title="Previous Variant"
            hint="Previous variant (↑)"
            onClick={() => {
              change("previous");
            }}
          />
          <span className="text-small font-semibold text-faint tabular-nums">{sides.position}</span>
          <Button
            variant="icon"
            icon={ChevronDown}
            title="Next Variant"
            hint="Next variant (↓)"
            onClick={() => {
              change("next");
            }}
          />
        </div>
      )}
      <Button
        variant="icon"
        icon={Maximize2}
        title="Fit"
        hint="Fit to pane"
        disabled={isFit(state)}
        onClick={() => {
          setState(fitState);
        }}
      />
      <Button
        variant="icon"
        icon={ZoomIn}
        title="Actual Size"
        hint="Show at 100%"
        disabled={isActualSize(state)}
        onClick={() => {
          setState(actualSizeState);
        }}
      />
      <input
        type="range"
        aria-label="Zoom"
        title="Zoom"
        min={minZoom}
        max={maxZoom}
        step="any"
        value={state.zoom}
        onChange={(event) => {
          const zoom = Number(event.currentTarget.value);
          setState((current) => withZoom(current, zoom));
        }}
        className="w-140 accent-accent"
      />
      <Button
        variant="icon"
        icon={ArrowLeftRight}
        title="Swap Sides"
        hint="Swap sides (← →)"
        onClick={() => {
          change("swap");
        }}
      />
    </div>
  );

  if (isError) {
    return (
      <div className="flex h-full flex-col gap-large px-page pt-page-top pb-page">
        {topBar}
        <EmptyState
          icon={CircleX}
          title="These images can't be compared."
          action={<Button variant="secondary" title="Library" onClick={() => void navigate({ to: "/library" })} />}
        />
      </div>
    );
  }
  return (
    <div className="flex h-full min-h-0 flex-col gap-large px-page pt-page-top pb-page">
      {topBar}
      {left && right ? (
        <>
          <div className="flex min-h-0 flex-1 gap-medium">
            <ImagePane key={left.id} image={left} state={state} setState={setState} />
            <ImagePane key={right.id} image={right} state={state} setState={setState} />
          </div>
          <div className="flex items-start justify-between gap-large">
            <p className="m-0 min-w-0 truncate text-body font-semibold text-text">{left.title}</p>
            <p className="m-0 min-w-0 truncate text-body font-semibold text-text">{right.title}</p>
          </div>
        </>
      ) : (
        <Spinner />
      )}
    </div>
  );
}

function fits(sides: ImageCompareSides, count: number): boolean {
  return sides.left < count && sides.right < count;
}

/**
 * One pane of the image compare: the picture at the shared zoom, panned with the shared offset. Dragging pans both
 * panes; pinching (Ctrl and the wheel, as a trackpad pinch arrives) zooms both. Fit shows the whole image; Actual Size
 * shows one image pixel per point.
 */
function ImagePane({
  image,
  state,
  setState,
}: {
  image: CompareImageView;
  state: ImageCompareState;
  setState: Dispatch<SetStateAction<ImageCompareState>>;
}) {
  const pane = useRef<HTMLDivElement>(null);
  const drag = useRef<{ x: number; y: number } | null>(null);
  const [paneSize, setPaneSize] = useState({ width: 1, height: 1 });
  const [imageSize, setImageSize] = useState<{ width: number; height: number } | null>(null);

  useEffect(() => {
    const element = pane.current;
    if (!element) return;
    const measure = () => {
      setPaneSize({ width: element.clientWidth, height: element.clientHeight });
    };
    measure();
    // The wheel is listened to directly: React's is passive, and a pinch must not zoom the whole window.
    const onWheel = (event: WheelEvent) => {
      if (!event.ctrlKey) return;
      event.preventDefault();
      const factor = Math.exp(-event.deltaY / 100);
      setState((current) => withZoom(current, current.zoom * factor));
    };
    element.addEventListener("wheel", onWheel, { passive: false });
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(element);
    return () => {
      element.removeEventListener("wheel", onWheel);
      observer?.disconnect();
    };
  }, [setState]);

  const size = imageSize ?? { width: 1, height: 1 };
  const scale = displayScale(state, fitScale(size.width, size.height, paneSize.width, paneSize.height));
  return (
    <div
      ref={pane}
      role="img"
      aria-label={image.title}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.currentTarget.setPointerCapture(event.pointerId);
        drag.current = { x: event.clientX, y: event.clientY };
      }}
      onPointerMove={(event) => {
        const last = drag.current;
        if (!last) return;
        // Only the step since the last event pans.
        const dx = event.clientX - last.x;
        const dy = event.clientY - last.y;
        drag.current = { x: event.clientX, y: event.clientY };
        setState((current) => panned(current, dx, dy));
      }}
      onPointerUp={() => {
        drag.current = null;
      }}
      onPointerCancel={() => {
        drag.current = null;
      }}
      className="relative min-w-0 flex-1 cursor-grab touch-none overflow-hidden rounded-card border border-stroke bg-well select-none active:cursor-grabbing"
    >
      {image.url !== null && (
        <img
          src={image.url}
          alt=""
          draggable={false}
          onLoad={(event) => {
            const { naturalWidth, naturalHeight } = event.currentTarget;
            setImageSize({ width: naturalWidth, height: naturalHeight });
          }}
          className={imageSize ? "absolute top-1/2 left-1/2 max-w-none" : "invisible absolute"}
          style={{
            width: size.width * scale,
            height: size.height * scale,
            transform: `translate(calc(-50% + ${String(state.offsetX)}px), calc(-50% + ${String(state.offsetY)}px))`,
          }}
        />
      )}
      {!imageSize && (
        <div className="absolute inset-0 flex items-center justify-center">
          <Spinner />
        </div>
      )}
    </div>
  );
}
