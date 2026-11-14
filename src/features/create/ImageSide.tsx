import { useQueryClient } from "@tanstack/react-query";
import { Image, Info, LoaderCircle, Sparkles, Trash2, TriangleAlert, X } from "lucide-react";
import { useEffect, useId, useRef, useState, type ReactNode } from "react";

import {
  commands,
  type ImageEdit_Deserialize,
  type ImageInputView_Serialize,
  type ImageSnapshot_Serialize,
  type ImageTile_Serialize,
  type UnsavedRecordsView,
} from "../../bindings";
import { AlertDialog } from "../../app/Dialog";
import { EmptyState } from "../../design/Components";
import { cx } from "../../design/cx";
import { HoverHint } from "../../design/HoverHint";
import { FilterMenu, SegmentedControl } from "../../design/Pickers";
import { RemoteThumbnail } from "../../design/RemoteThumbnail";
import { Spinner } from "../../design/Spinner";
import { Button, Card, focusRing, Well } from "../../design/Styles";
import { queryKeys } from "../../lib/queryKeys";
import { useShortcut } from "../../lib/shortcuts/useShortcut";
import { showError, useCreateNotice, useMediaUrl } from "./createData";
import { generateImages, useImagesEdit } from "./imageData";
import { PromptClearButton } from "./PromptHistory";
import { RequestRow } from "./RequestRow";

type ImageEdit = (edit: ImageEdit_Deserialize) => void;

/**
 * The Image side of Create (`CreateImageSide`): the form on the left with Generate at its bottom; the requests and
 * results on the right. A request keeps going while the side or the page changes, and every image is saved to
 * Library › Images as it arrives.
 */
export function ImageSide({ snapshot }: { snapshot: ImageSnapshot_Serialize }) {
  const edit = useImagesEdit();
  return (
    <div className="flex min-h-0 flex-1 gap-large">
      {/* The form fills the card, the prompt taking the height to spare; a window too short for it scrolls it. */}
      <Card className="flex w-380 min-w-320 shrink-0 flex-col overflow-y-auto">
        <div className="flex shrink-0 grow flex-col gap-18 p-large">
          <div className="flex flex-1 flex-col gap-18">
            {snapshot.unavailableReason !== null && snapshot.models.length === 0 ? (
              <p className="m-0 flex items-center gap-6 text-small text-muted">
                <Info aria-hidden size={14} />
                {snapshot.unavailableReason}
              </p>
            ) : (
              <ImageForm snapshot={snapshot} edit={edit} />
            )}
          </div>
          <GenerateSection snapshot={snapshot} />
        </div>
      </Card>
      <Card className="flex min-w-0 flex-1">
        <ImageStage snapshot={snapshot} />
      </Card>
      {snapshot.saveFailure && (
        <AlertDialog
          title={snapshot.saveFailure.title}
          message={snapshot.saveFailure.message}
          onCancel={() => void commands.imagesDismissSaveFailure()}
        >
          <Button variant="primary" title="OK" onClick={() => void commands.imagesDismissSaveFailure()} />
        </AlertDialog>
      )}
    </div>
  );
}

/** The prompt, then the model, frame, variants and image to edit as compact rows (`GenerateReferenceForm`). */
export function ImageForm({ snapshot, edit }: { snapshot: ImageSnapshot_Serialize; edit: ImageEdit }) {
  const id = useId();
  const field = useRef<HTMLTextAreaElement>(null);
  const [text, setText] = useState(snapshot.prompt);
  useEffect(() => {
    if (document.activeElement !== field.current) setText(snapshot.prompt);
  }, [snapshot.prompt]);
  const setPrompt = (value: string) => {
    setText(value);
    edit({ kind: "setPrompt", text: value });
  };
  return (
    <>
      <section className="flex flex-1 flex-col gap-small">
        <div className="flex min-h-34 items-center gap-small">
          <label htmlFor={id} className="text-label font-bold tracking-label text-faint uppercase">
            Prompt
          </label>
          {text !== "" && (
            <PromptClearButton
              clear={() => {
                setPrompt("");
              }}
            />
          )}
        </div>
        <textarea
          id={id}
          ref={field}
          value={text}
          aria-label="Prompt"
          className={cx(
            "min-h-160 w-full flex-1 resize-none rounded-field border border-stroke bg-well p-small text-body text-text",
            "outline-none focus:border-accent",
          )}
          onChange={(event) => {
            setPrompt(event.target.value);
          }}
        />
      </section>
      {snapshot.loadingModels ? <OptionSkeleton /> : <Options snapshot={snapshot} edit={edit} />}
    </>
  );
}

/** Where files dropped on the window become the image to edit (`CreatePage`'s drop routing). */
export const imageInputDropTarget = "imageInput";

function OptionRow({
  title,
  detail,
  drop,
  children,
}: {
  title: string;
  detail?: string;
  /** The `data-drop` target this row is. */
  drop?: string;
  children: ReactNode;
}) {
  return (
    <div
      data-drop={drop}
      className="flex min-h-44 items-center gap-medium border-b border-stroke py-6 last:border-b-0"
    >
      <span className="mr-auto flex min-w-0 flex-col gap-2">
        <span className="text-small text-text">{title}</span>
        {detail !== undefined && <span className="line-clamp-2 text-caption text-faint">{detail}</span>}
      </span>
      {children}
    </div>
  );
}

/** The rows' places while the connection's first list of models loads; the prompt can be written meanwhile. */
function OptionSkeleton() {
  return (
    <Well role="group" aria-label="Loading image models" className="px-medium">
      {(
        [
          ["Model", "w-180"],
          ["Frame", "w-90"],
          ["Variants", "w-150"],
        ] as const
      ).map(([title, width]) => (
        <OptionRow key={title} title={title}>
          <span aria-hidden className={cx("h-34 animate-pulse rounded-full bg-glass", width)} />
        </OptionRow>
      ))}
    </Well>
  );
}

function Options({ snapshot, edit }: { snapshot: ImageSnapshot_Serialize; edit: ImageEdit }) {
  return (
    <Well className="px-medium">
      <OptionRow title="Model">
        <FilterMenu
          title="Model"
          value={snapshot.modelId ?? ""}
          options={snapshot.models.map((model) => ({ value: model.id, title: model.title }))}
          onChange={(id) => {
            edit({ kind: "selectModel", id });
          }}
        />
      </OptionRow>
      <OptionRow title="Frame">
        <FilterMenu
          title="Frame"
          value={snapshot.aspectRatio ?? ""}
          options={snapshot.aspectRatios.map((ratio) => ({ value: ratio, title: ratio }))}
          onChange={(ratio) => {
            edit({ kind: "setAspectRatio", ratio });
          }}
        />
      </OptionRow>
      {snapshot.variantCounts.length > 1 && (
        <OptionRow title="Variants">
          <SegmentedControl
            isCompact
            aria-label="Variants"
            value={snapshot.count}
            options={snapshot.variantCounts.map((count) => ({ value: count, title: String(count) }))}
            onChange={(count) => {
              edit({ kind: "setCount", count });
            }}
          />
        </OptionRow>
      )}
      {snapshot.input && <InputRow input={snapshot.input} edit={edit} />}
    </Well>
  );
}

/** The image an edit model changes: optional for some models, required for others. */
function InputRow({ input, edit }: { input: ImageInputView_Serialize; edit: ImageEdit }) {
  const client = useQueryClient();
  const url = useMediaUrl(input.path);
  const choose = () => {
    commands.imagesChooseInput().then(
      (snapshot) => {
        client.setQueryData(queryKeys.images.snapshot, snapshot);
      },
      (error: unknown) => {
        showError("Can't add the image", error);
      },
    );
  };
  return (
    <OptionRow title="Image to edit" detail={input.detail} drop={imageInputDropTarget}>
      {input.path !== null ? (
        <span className="relative size-40">
          {url ? (
            <img src={url} alt={input.name ?? ""} className="size-40 rounded-hint object-cover" />
          ) : (
            <span className="block size-40 rounded-hint bg-glass" />
          )}
          <button
            type="button"
            aria-label="Remove"
            title="Remove"
            onClick={() => {
              edit({ kind: "setInput", path: null });
            }}
            className={cx(
              "absolute -top-8 -right-8 flex size-20 cursor-pointer items-center justify-center rounded-full",
              "border border-stroke bg-background text-text",
              focusRing,
            )}
          >
            <X aria-hidden size={10} strokeWidth={3} />
          </button>
        </span>
      ) : (
        <Button variant="secondary" title="Choose…" onClick={choose} />
      )}
    </OptionRow>
  );
}

/** What Generate's button can't say, then Generate with its price. */
function GenerateSection({ snapshot }: { snapshot: ImageSnapshot_Serialize }) {
  const { generate, notice } = snapshot;
  // The Generate shortcut and File › Generate run this side's Generate (`CreateView.menuSubmit`).
  useShortcut("generate", () => void generateImages(), generate.enabled, "Generate");
  return (
    <div className="flex flex-col gap-small">
      {notice && (
        <p
          className={cx(
            "m-0",
            notice.isQuiet ? "line-clamp-2 text-caption text-faint" : "text-body text-muted",
          )}
        >
          {notice.text}
        </p>
      )}
      <Button
        variant="primary"
        className="w-full"
        icon={generate.isGenerating ? LoaderCircle : Sparkles}
        title={generate.isGenerating ? "Generating…" : "Generate"}
        price={generate.isGenerating ? undefined : (generate.price ?? undefined)}
        hint={generate.hint === "" ? undefined : generate.hint}
        disabled={!generate.enabled}
        onClick={() => void generateImages()}
      />
    </div>
  );
}

/**
 * The right side: the picked result large (the newest without a pick) over a strip of the results, with the requests
 * that are running or didn't finish. Before a request's first image arrives, it is a spinner with Cancel.
 */
function ImageStage({ snapshot }: { snapshot: ImageSnapshot_Serialize }) {
  const cancel = (id: string) => void commands.imagesCancel(id);
  const runs = snapshot.runs.map((run) => (
    <RequestRow
      key={run.id}
      row={run}
      onCancel={() => {
        cancel(run.id);
      }}
    />
  ));
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-medium p-large">
      {snapshot.unsaved && <UnsavedRecordsWarning unsaved={snapshot.unsaved} />}
      {snapshot.current ? (
        <>
          <div className="flex min-h-0 flex-1 items-center justify-center overflow-hidden rounded-small bg-well">
            <RemoteThumbnail url={snapshot.current.url} contentMode="fit" />
          </div>
          <p className="m-0 line-clamp-2 text-small text-muted select-text">{snapshot.current.prompt}</p>
          {runs}
          <Strip snapshot={snapshot} />
        </>
      ) : snapshot.running ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-medium">
          <Spinner label={snapshot.running.title} />
          <p className="m-0 text-body text-muted">{snapshot.running.title}</p>
          <Button
            variant="secondary"
            title="Cancel"
            onClick={() => {
              if (snapshot.running) cancel(snapshot.running.id);
            }}
          />
        </div>
      ) : (
        <>
          {runs}
          <EmptyState icon={Image} title="No images yet" className="my-auto" />
        </>
      )}
    </div>
  );
}

/** The results, newest first, after the places of the images running requests will make. */
function Strip({ snapshot }: { snapshot: ImageSnapshot_Serialize }) {
  const client = useQueryClient();
  const refresh = () => client.invalidateQueries({ queryKey: queryKeys.images.all });
  return (
    <div className="flex h-72 items-center gap-medium">
      <div className="flex min-w-0 flex-1 gap-small overflow-x-auto">
        {Array.from({ length: snapshot.pendingTiles }, (_, index) => (
          <span
            key={`pending-${String(index)}`}
            role="img"
            aria-label="Image being made"
            className="flex size-72 shrink-0 items-center justify-center rounded-small border border-dashed border-stroke bg-well"
          >
            <Spinner />
          </span>
        ))}
        {snapshot.images.map((image) => (
          <StripTile
            key={image.id}
            image={image}
            isSelected={snapshot.current?.id === image.id}
            onSelect={() => void commands.imagesShow(image.id).then(refresh)}
            onDelete={() => {
              commands.imagesDelete(image.id).then(
                (notice) => {
                  if (notice) useCreateNotice.getState().show(notice);
                  void refresh();
                },
                (error: unknown) => {
                  showError("Couldn't move the image to the Trash", error);
                },
              );
            }}
          />
        ))}
      </div>
      <HoverHint text="Clear list">
        <Button variant="textLink" title="Clear" onClick={() => void commands.imagesClear().then(refresh)} />
      </HoverHint>
    </div>
  );
}

/** One result in the strip: a click shows it large, the trash button moves it to the Trash. */
function StripTile({
  image,
  isSelected,
  onSelect,
  onDelete,
}: {
  image: ImageTile_Serialize;
  isSelected: boolean;
  onSelect: () => void;
  onDelete: () => void;
}) {
  return (
    <HoverHint text={image.prompt} className="shrink-0">
      <div
        className={cx(
          "group relative size-72 overflow-hidden rounded-small bg-well",
          isSelected ? "border-2 border-accent" : "border border-stroke",
        )}
      >
        <button
          type="button"
          aria-label={image.prompt}
          aria-pressed={isSelected}
          onClick={onSelect}
          className={cx("size-full cursor-pointer", focusRing)}
        >
          <RemoteThumbnail url={image.url} />
        </button>
        {!image.isPicked && (
          <button
            type="button"
            aria-label="Move to Trash"
            title="Move to Trash"
            onClick={onDelete}
            className={cx(
              "absolute top-3 right-3 hidden size-22 cursor-pointer items-center justify-center rounded-full",
              "bg-scrim text-text group-hover:flex group-focus-within:flex",
              focusRing,
            )}
          >
            <Trash2 aria-hidden size={11} />
          </button>
        )}
      </div>
    </HoverHint>
  );
}

/**
 * Shown while the prices and jobs of image requests can't be written down: Try Again writes them again; Copy Job IDs
 * keeps the jobs Higgsfield accepted, so they can be found in the account's history after a quit.
 */
export function UnsavedRecordsWarning({ unsaved }: { unsaved: UnsavedRecordsView }) {
  return (
    <Well className="flex flex-col gap-6 px-medium py-10">
      <p className="m-0 flex items-center gap-6 text-caption text-warning">
        <TriangleAlert aria-hidden size={12} />
        {unsaved.reason}
      </p>
      <p className="m-0 text-caption text-muted">{unsaved.detail}</p>
      <div className="flex gap-small">
        <Button variant="secondary" title="Try Again" onClick={() => void commands.imagesRetrySaving()} />
        {unsaved.jobIds.length > 0 && (
          <Button
            variant="secondary"
            title="Copy Job IDs"
            onClick={() => void navigator.clipboard.writeText(unsaved.jobIds.join("\n"))}
          />
        )}
      </div>
    </Well>
  );
}
