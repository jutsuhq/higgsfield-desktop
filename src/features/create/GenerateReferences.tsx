import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, ChevronLeft, ChevronRight, Info, Maximize2, Plus, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";

import {
  commands,
  events,
  type AppError,
  type ImageSnapshot_Serialize,
  type ImageTile_Serialize,
  type ReferencePicksView_Serialize,
} from "../../bindings";
import { AlertDialog } from "../../app/Dialog";
import { SectionLabel } from "../../design/Components";
import { cx } from "../../design/cx";
import { HoverHint } from "../../design/HoverHint";
import { RemoteThumbnail } from "../../design/RemoteThumbnail";
import { Spinner } from "../../design/Spinner";
import { Button, focusRing } from "../../design/Styles";
import { useInvalidateOn } from "../../lib/events";
import { queryKeys } from "../../lib/queryKeys";
import { showError, useCreateNotice } from "./createData";
import { GenerateButton, GenerateSheetLayout, PendingTile } from "./GenerateSheet";
import { generateImages, useImagesEdit, useImagesSnapshot } from "./imageData";
import { ImageForm, UnsavedRecordsWarning } from "./ImageSide";
import { RequestRow } from "./RequestRow";

/** Generate…'s picks (`images_reference_picks`); a result or a change of the references asks for them again. */
export function useReferencePicks() {
  useInvalidateOn(events.imagesChanged, [queryKeys.images.referencePicks]);
  useInvalidateOn(events.createChanged, [queryKeys.images.referencePicks]);
  return useQuery<ReferencePicksView_Serialize, AppError>({
    queryKey: queryKeys.images.referencePicks,
    queryFn: () => commands.imagesReferencePicks(),
  });
}

/**
 * References › Generate… (`GenerateReferenceSheet`): makes reference images with the connection's image models. The
 * form is on the left and the results on the right. A click picks a result, a double click opens it large, and Add
 * puts the picked ones into References in the order they were picked, then closes. While a request runs, the form and
 * the cross wait for it. The form, runs and results are the Image side's, so they outlive the sheet; closing only
 * drops the picks.
 */
export function GenerateReferenceSheet({ onClose }: { onClose: () => void }) {
  const snapshot = useImagesSnapshot().data;
  const picks = useReferencePicks().data;
  const client = useQueryClient();
  const edit = useImagesEdit();
  const [adding, setAdding] = useState<string | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [isConfirmingClose, setIsConfirmingClose] = useState(false);
  const [previewed, setPreviewed] = useState<string | null>(null);
  useEffect(
    () => () => {
      void commands.imagesClearReferencePicks();
    },
    [],
  );
  const isGenerating = snapshot?.generate.isGenerating ?? false;
  const isAdding = adding !== null;
  const isBusy = isGenerating || isAdding;
  const picked = picks?.picked ?? [];

  const toggle = (id: string) => {
    commands.imagesToggleReferencePick(id).then(
      (next) => {
        client.setQueryData(queryKeys.images.referencePicks, next);
      },
      (error: unknown) => {
        showError("Can't pick the image", error);
      },
    );
  };
  const generate = () => {
    if (!isAdding) void generateImages();
  };
  const remove = (id: string) => {
    if (previewed === id) setPreviewed(null);
    commands.imagesDelete(id).then(
      (notice) => {
        if (notice) useCreateNotice.getState().show(notice);
      },
      (error: unknown) => {
        showError("Can't move the image to the Trash", error);
      },
    );
  };
  // Adds the picks in the order they were picked, then closes; a failure stops there and keeps the rest picked.
  const addPicks = async () => {
    if (isBusy) return;
    setFailure(null);
    for (const id of picked) {
      setAdding(id);
      try {
        const message = await commands.createAddGenerated(id);
        if (message !== null) {
          setFailure(message);
          setAdding(null);
          return;
        }
      } catch (error) {
        showError("Can't add the image", error);
        setAdding(null);
        return;
      }
    }
    setAdding(null);
    onClose();
  };
  // Picked images not added yet are confirmed first; everything else just closes.
  const close = () => {
    if (picked.length === 0) onClose();
    else setIsConfirmingClose(true);
  };

  return (
    <GenerateSheetLayout
      title="Generate reference images"
      subtitle="Every image is saved to Library › Images. Pick the ones to add to the references."
      isBusy={isBusy}
      closeHint={isGenerating ? "Wait for the images" : "Close"}
      closesOnEscape={previewed === null && !isConfirmingClose}
      onClose={close}
      form={
        snapshot === undefined ? (
          <Spinner />
        ) : snapshot.unavailableReason !== null && snapshot.models.length === 0 ? (
          <p className="m-0 flex items-center gap-6 text-body text-muted">
            <Info aria-hidden size={14} />
            {snapshot.unavailableReason}
          </p>
        ) : (
          <div className="flex min-h-full flex-col gap-18">
            <ImageForm snapshot={snapshot} edit={edit} />
          </div>
        )
      }
      results={
        snapshot && (
          <Results
            snapshot={snapshot}
            picks={picks}
            adding={adding}
            onToggle={toggle}
            onPreview={setPreviewed}
            onDelete={remove}
          />
        )
      }
      actions={
        snapshot && (
          <>
            {failure !== null ? (
              <p className="m-0 line-clamp-2 text-small font-medium text-muted">{failure}</p>
            ) : (
              snapshot.notice && (
                <p
                  className={cx(
                    "m-0",
                    snapshot.notice.isQuiet ? "line-clamp-2 text-caption text-faint" : "text-body text-muted",
                  )}
                >
                  {snapshot.notice.text}
                </p>
              )
            )}
            <span className="flex-1" />
            <GenerateButton
              isGenerating={isGenerating}
              price={isGenerating ? null : snapshot.generate.price}
              isPrimary={picked.length === 0}
              hint={snapshot.generate.hint}
              disabled={!snapshot.generate.enabled || isBusy}
              onClick={generate}
            />
            {picked.length > 0 && (
              <Button
                variant="primary"
                icon={isAdding ? undefined : Plus}
                title={isAdding ? "Adding…" : `Add ${String(picked.length)} to References`}
                hint={isGenerating ? "Wait for the images" : undefined}
                disabled={isBusy}
                onClick={() => void addPicks()}
              />
            )}
          </>
        )
      }
    >
      {previewed !== null && snapshot && (
        <Preview
          images={snapshot.images}
          current={previewed}
          picks={picks}
          onMove={setPreviewed}
          onToggle={toggle}
          onClose={() => {
            setPreviewed(null);
          }}
        />
      )}
      {isConfirmingClose && (
        <AlertDialog
          title="Close without adding the picked images?"
          message="They stay in Library › Images, where you can add them later."
          onCancel={() => {
            setIsConfirmingClose(false);
          }}
        >
          <Button
            variant="secondary"
            title="Keep Picking"
            onClick={() => {
              setIsConfirmingClose(false);
            }}
          />
          <Button variant="secondary" destructive title="Close" onClick={onClose} />
        </AlertDialog>
      )}
    </GenerateSheetLayout>
  );
}

function pickNumber(picks: ReferencePicksView_Serialize | undefined, id: string): number | undefined {
  const index = picks?.picked.indexOf(id) ?? -1;
  return index >= 0 ? index + 1 : undefined;
}

function Results({
  snapshot,
  picks,
  adding,
  onToggle,
  onPreview,
  onDelete,
}: {
  snapshot: ImageSnapshot_Serialize;
  picks: ReferencePicksView_Serialize | undefined;
  adding: string | null;
  onToggle: (id: string) => void;
  onPreview: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-small">
      <SectionLabel title="Results" detail={picks?.detail ?? undefined} />
      {snapshot.unsaved && <UnsavedRecordsWarning unsaved={snapshot.unsaved} />}
      <div className="flex min-h-0 flex-1 flex-col gap-medium overflow-y-auto">
        {/* A finished request shows as its images; a running, failed or canceled one keeps its row. */}
        {snapshot.runs.map((run) => (
          <RequestRow key={run.id} row={run} onCancel={() => void commands.imagesCancel(run.id)} />
        ))}
        {snapshot.pendingTiles === 0 && snapshot.images.length === 0 && (
          <p className="m-0 flex min-h-240 items-center justify-center text-center text-body text-faint">
            The images you generate show here. Click one to pick it, double-click to open it large.
          </p>
        )}
        <div className="grid grid-cols-[repeat(auto-fill,minmax(140px,1fr))] gap-small">
          {Array.from({ length: snapshot.pendingTiles }, (_, index) => (
            <PendingTile key={`pending-${String(index)}`} />
          ))}
          {snapshot.images.map((image) => (
            <ResultTile
              key={image.id}
              image={image}
              pickNumber={pickNumber(picks, image.id)}
              isAdded={picks?.added.includes(image.id) ?? false}
              canPick={picks?.canPick ?? false}
              isAdding={adding === image.id}
              onToggle={() => {
                onToggle(image.id);
              }}
              onPreview={() => {
                onPreview(image.id);
              }}
              onDelete={() => {
                onDelete(image.id);
              }}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

/**
 * A generated image in the results (`GeneratedResultTile`). A click picks it, with its place among the picks as the
 * badge; a double click or the expand button opens it large, the trash button deletes it. An image already added
 * shows Added and can't be picked again or deleted.
 */
function ResultTile({
  image,
  pickNumber,
  isAdded,
  canPick,
  isAdding,
  onToggle,
  onPreview,
  onDelete,
}: {
  image: ImageTile_Serialize;
  pickNumber: number | undefined;
  isAdded: boolean;
  canPick: boolean;
  isAdding: boolean;
  onToggle: () => void;
  onPreview: () => void;
  onDelete: () => void;
}) {
  const isSelectable = !isAdded && (pickNumber !== undefined || canPick);
  const label = isAdded
    ? "Generated image, added to the references"
    : pickNumber !== undefined
      ? `Generated image, picked ${String(pickNumber)}`
      : "Generated image";
  return (
    <HoverHint text={image.prompt}>
      <div
        className={cx(
          "group relative aspect-square w-full overflow-hidden rounded-small bg-well",
          pickNumber !== undefined ? "border-2 border-accent" : "border border-stroke",
          !(isSelectable || pickNumber !== undefined || isAdded) && "opacity-45",
        )}
      >
        <button
          type="button"
          aria-label={label}
          aria-pressed={pickNumber !== undefined}
          className={cx("absolute inset-0 block size-full cursor-pointer", focusRing)}
          onClick={() => {
            if (isSelectable) onToggle();
          }}
          onDoubleClick={onPreview}
        >
          <RemoteThumbnail url={image.url} className="absolute inset-0 size-full" />
        </button>
        <span className="pointer-events-none absolute top-6 left-6">
          {isAdded ? (
            <span className="flex h-20 items-center gap-2 rounded-full bg-accent px-7 text-micro font-heavy text-ink">
              <Check aria-hidden size={10} strokeWidth={3} />
              Added
            </span>
          ) : pickNumber !== undefined ? (
            <span className="flex size-20 items-center justify-center rounded-full bg-accent text-micro font-heavy text-ink tabular-nums">
              {pickNumber}
            </span>
          ) : (
            isSelectable && (
              <span className="hidden size-20 rounded-full border-[1.5px] border-text bg-scrim group-hover:block" />
            )
          )}
        </span>
        <span className="absolute top-3 right-3 hidden group-focus-within:flex group-hover:flex">
          {!isAdded && !isAdding && (
            <Button
              variant="icon"
              icon={Trash2}
              title="Move to Trash"
              hint="Move to Trash"
              onClick={onDelete}
            />
          )}
          <Button variant="icon" icon={Maximize2} title="Open large" hint="Open large" onClick={onPreview} />
        </span>
        {isAdding && (
          <span className="absolute inset-0 flex items-center justify-center">
            <Spinner />
          </span>
        )}
      </div>
    </HoverHint>
  );
}

/**
 * A result open large over the sheet (`GeneratedImagePreview`): ← and → step through the results, Space picks or
 * unpicks, Escape goes back to the grid.
 */
function Preview({
  images,
  current,
  picks,
  onMove,
  onToggle,
  onClose,
}: {
  images: ImageTile_Serialize[];
  current: string;
  picks: ReferencePicksView_Serialize | undefined;
  onMove: (id: string) => void;
  onToggle: (id: string) => void;
  onClose: () => void;
}) {
  const index = images.findIndex((image) => image.id === current);
  const image = images[index];
  const number = image === undefined ? undefined : pickNumber(picks, image.id);
  const isAdded = image !== undefined && (picks?.added.includes(image.id) ?? false);
  const isSelectable = !isAdded && (number !== undefined || (picks?.canPick ?? false));
  const previous = images[index - 1];
  const next = images[index + 1];
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onClose();
      } else if (event.key === "ArrowLeft" && previous) {
        onMove(previous.id);
      } else if (event.key === "ArrowRight" && next) {
        onMove(next.id);
      } else if (event.key === " " && image && isSelectable) {
        event.preventDefault();
        onToggle(image.id);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
    };
  }, [image, isSelectable, next, onClose, onMove, onToggle, previous]);
  if (image === undefined) return null;
  return (
    <div
      role="dialog"
      aria-label="Generated image"
      className="absolute inset-0 z-10 flex flex-col gap-medium rounded-card bg-background p-x-large"
    >
      <div className="flex items-baseline gap-small">
        <span className="text-icon font-bold text-text">
          {`Image ${String(index + 1)} of ${String(images.length)}`}
        </span>
        <span className="text-body text-muted">{image.model}</span>
      </div>
      <div
        role="img"
        aria-label={image.prompt}
        className="flex min-h-0 flex-1 items-center justify-center overflow-hidden rounded-small bg-well"
      >
        <RemoteThumbnail url={image.url} contentMode="fit" />
      </div>
      <p className="m-0 line-clamp-2 text-caption text-faint select-text">{image.prompt}</p>
      <div className="flex items-center gap-small">
        <Button
          variant="icon"
          icon={ChevronLeft}
          title="Previous"
          disabled={previous === undefined}
          onClick={() => {
            if (previous) onMove(previous.id);
          }}
        />
        <Button
          variant="icon"
          icon={ChevronRight}
          title="Next"
          disabled={next === undefined}
          onClick={() => {
            if (next) onMove(next.id);
          }}
        />
        <span className="flex-1" />
        <Button variant="secondary" title="Back to Results" onClick={onClose} />
        <Button
          variant="primary"
          icon={isAdded || number !== undefined ? Check : Plus}
          title={isAdded ? "Added" : number !== undefined ? `Picked ${String(number)}` : "Pick"}
          hint={number === undefined ? "Add to references" : "Unpick"}
          disabled={!isSelectable}
          onClick={() => {
            onToggle(image.id);
          }}
        />
      </div>
    </div>
  );
}
