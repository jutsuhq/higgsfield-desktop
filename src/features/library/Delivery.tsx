import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Channel } from "@tauri-apps/api/core";
import {
  CircleAlert,
  CircleCheck,
  CircleMinus,
  Clock,
  Film,
  Folder,
  GitBranch,
  Image,
  Lock,
  LockOpen,
  MinusCircle,
  Package,
  Trash2,
} from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";

import {
  commands,
  type CropFrame,
  type DeliveryChoice,
  type DeliveryExportScope,
  type DeliveryItemView_Serialize,
  type DeliveryOutputChange,
  type DeliveryOutputOptions,
  type DeliveryOutputView_Serialize,
  type DeliverySetChange,
  type DeliverySetView_Serialize,
  type DeliveryTaskUpdate,
} from "../../bindings";
import { AlertDialog, Dialog } from "../../app/Dialog";
import { EmptyState, SectionLabel } from "../../design/Components";
import { cx } from "../../design/cx";
import { HoverHint } from "../../design/HoverHint";
import { FilterMenu } from "../../design/Pickers";
import { Spinner } from "../../design/Spinner";
import { Button, focusRing, TextField } from "../../design/Styles";
import { queryKeys } from "../../lib/queryKeys";
import { ContextMenu } from "./LibraryDialogs";
import { useThumbnail } from "./libraryData";

const deliveryKey = [...queryKeys.library.all, "delivery"] as const;
const setsKey = [...deliveryKey, "sets"] as const;
const setKey = (id: string) => [...deliveryKey, "set", id] as const;

/** `DeliveryContent.title`, the Library header's button and the sheet's title. */
export const DELIVERY_TITLE = "Delivery Sets";
export const NEW_SET_TITLE = "New Delivery Set";

/**
 * Library › Delivery Sets (DeliverySetsSheet.swift): every set on the left, the chosen one on the right. A sheet over
 * the Library, since a set is made from the Library's selection and its items are the Library's versions.
 */
export function DeliverySetsSheet({ initial, onClose }: { initial: string | null; onClose: () => void }) {
  const rows = useQuery({ queryKey: setsKey, queryFn: () => commands.deliverySets() }).data;
  const [selection, setSelection] = useState<string | null>(initial);
  const [isExporting, setIsExporting] = useState(false);
  const shown = selection ?? rows?.[0]?.id ?? null;

  return (
    <Dialog title={DELIVERY_TITLE} onCancel={isExporting ? () => undefined : onClose} className="h-640 max-w-940 flex-row gap-0 p-0">
      <div className="flex w-230 shrink-0 flex-col gap-small border-r border-stroke">
        <h2 className="m-0 px-large pt-large text-heading font-bold">{DELIVERY_TITLE}</h2>
        <ul role="listbox" aria-label={DELIVERY_TITLE} className="m-0 flex min-h-0 flex-1 list-none flex-col gap-2 overflow-y-auto p-small">
          {rows?.map((row) => (
            <li key={row.id}>
              <button
                type="button"
                role="option"
                aria-selected={row.id === shown}
                onClick={() => {
                  setSelection(row.id);
                }}
                className={cx(
                  "flex w-full cursor-pointer flex-col gap-2 rounded-field px-medium py-small text-left",
                  row.id === shown ? "bg-glass" : "hovered:bg-glass",
                  focusRing,
                )}
              >
                <span className="truncate text-control font-semibold text-text">{row.name}</span>
                <span className="truncate text-caption text-faint">{row.status}</span>
              </button>
            </li>
          ))}
        </ul>
        <div className="flex justify-end p-large">
          <Button variant="primary" title="Done" disabled={isExporting} onClick={onClose} />
        </div>
      </div>
      <div className="flex min-w-0 flex-1 flex-col">
        {shown === null ? (
          <EmptyState
            icon={Package}
            title="No delivery set selected"
            detail="Select videos or images in the Library and choose New Delivery Set to fix their versions for a delivery."
            className="my-auto"
          />
        ) : (
          <DeliverySetDetail
            key={shown}
            id={shown}
            onSelect={setSelection}
            onExporting={setIsExporting}
            onDeleted={() => {
              setSelection(rows?.find((row) => row.id !== shown)?.id ?? null);
            }}
          />
        )}
      </div>
    </Dialog>
  );
}

/** Saves one edit; Rust applies and saves edits one at a time and alerts on a failure. */
function useChange(id: string) {
  const client = useQueryClient();
  return (change: DeliverySetChange) => {
    commands
      .deliveryChange(id, change)
      .catch(() => undefined)
      .finally(() => {
        void client.invalidateQueries({ queryKey: deliveryKey });
      });
  };
}

/**
 * One delivery set (DeliverySetDetail.swift): its exact versions, outputs, status, note and review link, and the
 * export actions. Every edit is saved as it is made; once exported, the items stay as they are.
 */
function DeliverySetDetail({
  id,
  onSelect,
  onExporting,
  onDeleted,
}: {
  id: string;
  onSelect: (id: string) => void;
  onExporting: (isExporting: boolean) => void;
  onDeleted: () => void;
}) {
  const client = useQueryClient();
  const query = useQuery({ queryKey: setKey(id), queryFn: () => commands.deliverySet(id) });
  const view = query.data;
  const change = useChange(id);
  const [framing, setFraming] = useState<{ versionId: string; aspect: string } | null>(null);
  const [confirmsDelete, setConfirmsDelete] = useState(false);
  const [problems, setProblems] = useState<string | null>(null);
  const isExporting = view?.isExporting === true;
  useEffect(() => {
    onExporting(isExporting);
  }, [isExporting, onExporting]);

  if (query.isError) {
    return (
      <EmptyState icon={CircleAlert} title="Can't open this delivery set" detail={query.error.message} className="my-auto" />
    );
  }
  if (!view) return null;

  const refresh = () => void client.invalidateQueries({ queryKey: deliveryKey });
  const exportFiles = (scope: DeliveryExportScope) => {
    const onTask = new Channel<DeliveryTaskUpdate>();
    onTask.onmessage = refresh;
    const run = commands.deliveryExport(id, scope, onTask);
    // The run's state comes from Rust; the first read shows it running.
    setTimeout(refresh, 0);
    run
      .then((summary) => {
        if (summary?.problems != null) setProblems(summary.problems);
      })
      .catch(() => undefined)
      .finally(refresh);
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex min-h-0 flex-1 flex-col gap-x-large overflow-y-auto p-large">
        <Header view={view} onRename={(name) => { change({ kind: "rename", name }); }} />
        <section className="flex flex-col gap-small">
          <SectionLabel title="Items" detail={String(view.items.length)} />
          <div className="grid grid-cols-[repeat(auto-fill,128px)] gap-medium">
            {view.items.map((item) => (
              <ItemTile
                key={item.versionId}
                item={item}
                isFrozen={view.isFrozen}
                isDisabled={isExporting}
                onFrame={(aspect) => {
                  setFraming({ versionId: item.versionId, aspect });
                }}
                onRemove={() => {
                  change({ kind: "removeItem", version_id: item.versionId });
                }}
              />
            ))}
          </div>
        </section>
        <section className="flex flex-col gap-small">
          <SectionLabel title="Outputs" detail={String(view.outputs.length)} />
          {view.outputs.map((output) => (
            <OutputRow
              key={output.id}
              output={output}
              options={view.options}
              isDisabled={isExporting}
              onChange={(next) => {
                change({ kind: "output", output_id: output.id, change: next });
              }}
              onRemove={() => {
                change({ kind: "removeOutput", output_id: output.id });
              }}
            />
          ))}
          <div className="flex gap-small">
            <Button
              variant="secondary"
              icon={Image}
              title="Add Image Output"
              disabled={isExporting}
              onClick={() => {
                change({ kind: "addOutput", is_video: false });
              }}
            />
            <Button
              variant="secondary"
              icon={Film}
              title="Add Video Output"
              disabled={isExporting}
              onClick={() => {
                change({ kind: "addOutput", is_video: true });
              }}
            />
          </div>
        </section>
        <section className="flex flex-col gap-small">
          <SectionLabel title="Note" />
          <SavedText
            label="Note"
            placeholder="What this round is for, what changed"
            value={view.note}
            isMultiline
            onSave={(note) => {
              change({ kind: "note", note });
            }}
          />
          <SectionLabel title="Review link" />
          <SavedText
            label="Review link"
            placeholder="Where the client reviews it (text only)"
            value={view.reviewLink}
            onSave={(link) => {
              change({ kind: "reviewLink", link });
            }}
          />
        </section>
      </div>
      <Footer
        view={view}
        onNextRound={() => {
          commands
            .deliveryNextRound(id)
            .then((next) => {
              refresh();
              onSelect(next);
            })
            .catch(() => undefined);
        }}
        onShowFolder={() => void commands.deliveryShowFolder(id).catch(() => undefined)}
        onDelete={() => {
          setConfirmsDelete(true);
        }}
        onExport={exportFiles}
        onCancel={() => void commands.deliveryCancel()}
      />
      {framing && (
        <FrameSheet
          id={id}
          versionId={framing.versionId}
          aspect={framing.aspect}
          onDone={(frame) => {
            change({ kind: "frame", version_id: framing.versionId, aspect: framing.aspect, frame });
            setFraming(null);
          }}
          onCancel={() => {
            setFraming(null);
          }}
        />
      )}
      {confirmsDelete && (
        <AlertDialog
          title={`Delete “${view.name}”?`}
          message="Exported files and the library's versions stay where they are."
          onCancel={() => {
            setConfirmsDelete(false);
          }}
        >
          <Button
            variant="secondary"
            title="Cancel"
            onClick={() => {
              setConfirmsDelete(false);
            }}
          />
          <Button
            variant="secondary"
            destructive
            title="Delete Set"
            onClick={() => {
              setConfirmsDelete(false);
              commands
                .deliveryDelete(id)
                .then(() => {
                  onDeleted();
                  refresh();
                })
                .catch(() => undefined);
            }}
          />
        </AlertDialog>
      )}
      {problems !== null && (
        <AlertDialog
          title="Some files weren't exported"
          message={problems}
          onCancel={() => {
            setProblems(null);
          }}
        >
          <Button
            variant="secondary"
            title="OK"
            onClick={() => {
              setProblems(null);
            }}
          />
        </AlertDialog>
      )}
    </div>
  );
}

function Header({ view, onRename }: { view: DeliverySetView_Serialize; onRename: (name: string) => void }) {
  const LockIcon = view.isFrozen ? Lock : LockOpen;
  return (
    <div className="flex flex-col gap-small">
      <SavedText label="Set name" placeholder="Set name" value={view.name} onSave={onRename} />
      <HoverHint text={view.lockHint}>
        <span className="flex items-center gap-small text-caption">
        <LockIcon aria-hidden size={12} className={view.isFrozen ? "text-accent" : "text-faint"} />
        <span className="text-muted">{view.status}</span>
        {view.counts !== null && (
          <>
            <span className="text-faint">·</span>
            <span className="text-muted">{view.counts}</span>
          </>
        )}
        </span>
      </HoverHint>
    </div>
  );
}

/** A text field whose every edit is saved, as `.onChange(of: draft)` saves the set. */
function SavedText({
  label,
  placeholder,
  value,
  isMultiline = false,
  onSave,
}: {
  label: string;
  placeholder: string;
  value: string;
  isMultiline?: boolean;
  onSave: (text: string) => void;
}) {
  const [text, setText] = useState(value);
  const isEditing = useRef(false);
  useEffect(() => {
    if (!isEditing.current) setText(value);
  }, [value]);
  const props = {
    "aria-label": label,
    placeholder,
    value: text,
    onFocus: () => {
      isEditing.current = true;
    },
    onBlur: () => {
      isEditing.current = false;
    },
    onChange: (event: { target: { value: string } }) => {
      setText(event.target.value);
      onSave(event.target.value);
    },
  };
  return isMultiline ? (
    <textarea
      {...props}
      rows={2}
      className={cx(
        "w-full resize-y rounded-field border border-stroke bg-well px-medium py-small text-body text-text",
        focusRing,
      )}
    />
  ) : (
    <TextField {...props} />
  );
}

/** One item: its picture with the run badge, its name, and Frame and Remove. */
function ItemTile({
  item,
  isFrozen,
  isDisabled,
  onFrame,
  onRemove,
}: {
  item: DeliveryItemView_Serialize;
  isFrozen: boolean;
  isDisabled: boolean;
  onFrame: (aspect: string) => void;
  onRemove: () => void;
}) {
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [first] = item.frameAspects;
  return (
    <div className="flex w-128 flex-col gap-4">
      <div className="relative h-96 w-128 overflow-hidden rounded-small bg-well">
        <Preview item={item} />
        <div className="absolute top-6 right-6">
          <Badge item={item} />
        </div>
      </div>
      <span className="truncate text-caption text-text">{item.name}</span>
      <div className="flex items-center gap-4 text-caption font-semibold text-accent">
        {item.frameAspects.length === 1 && first !== undefined && (
          <TextAction
            title={`Frame ${first.title}…`}
            disabled={isDisabled}
            onClick={() => {
              onFrame(first.value);
            }}
          />
        )}
        {item.frameAspects.length > 1 && (
          <TextAction
            title="Frame…"
            disabled={isDisabled}
            onClick={(event) => {
              setMenu({ x: event.clientX, y: event.clientY });
            }}
          />
        )}
        {!isFrozen && (
          <HoverHint text="Remove from set">
            <button
              type="button"
              aria-label="Remove"
              disabled={isDisabled}
              onClick={onRemove}
              className={cx("flex cursor-pointer text-accent disabled:opacity-45", focusRing)}
            >
              <MinusCircle aria-hidden size={13} />
            </button>
          </HoverHint>
        )}
      </div>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={item.frameAspects.map((aspect: DeliveryChoice) => ({
            title: aspect.title,
            run: () => {
              onFrame(aspect.value);
            },
          }))}
          onClose={() => {
            setMenu(null);
          }}
        />
      )}
    </div>
  );
}

function TextAction({
  title,
  disabled,
  onClick,
}: {
  title: string;
  disabled: boolean;
  onClick: (event: React.MouseEvent) => void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={cx("cursor-pointer text-accent disabled:opacity-45", focusRing)}
    >
      {title}
    </button>
  );
}

function Preview({ item }: { item: DeliveryItemView_Serialize }) {
  const frame = useThumbnail(item.materialId, item.isVideo && item.preview.exists).data ?? null;
  const source = item.isVideo ? frame : item.preview.url;
  if (source === null) return null;
  return <img src={source} alt="" className="size-full object-cover" />;
}

/** Done, failed, skipped or running, over an item's picture (`DeliveryItemBadge`). */
function Badge({ item }: { item: DeliveryItemView_Serialize }) {
  const state = item.state;
  if (state === null) return null;
  if (state.kind === "running") return <Spinner />;
  const badges: Record<typeof state.kind, readonly [typeof Clock, string]> = {
    waiting: [Clock, "text-faint"],
    done: [CircleCheck, "text-accent"],
    failed: [CircleAlert, "text-danger"],
    skipped: [CircleMinus, "text-warning"],
  };
  const [Icon, color] = badges[state.kind];
  return (
    <HoverHint text={item.stateHint ?? undefined}>
      <span className="flex rounded-full bg-background">
        <Icon aria-label={item.stateHint ?? state.kind} size={15} strokeWidth={2.5} className={color} />
      </span>
    </HoverHint>
  );
}

/**
 * One output (DeliveryOutputEditing.swift): its name in the file names, its preset and the aspect it crops to. Images
 * choose format, size, quality, color space and, for formats without alpha, the background.
 */
function OutputRow({
  output,
  options,
  isDisabled,
  onChange,
  onRemove,
}: {
  output: DeliveryOutputView_Serialize;
  options: DeliveryOutputOptions;
  isDisabled: boolean;
  onChange: (change: DeliveryOutputChange) => void;
  onRemove: () => void;
}) {
  const menu = (title: string, value: string, choices: DeliveryChoice[], field: Exclude<DeliveryOutputChange["field"], "quality">) => (
    <FilterMenu
      title={title}
      value={value}
      options={choices}
      onChange={(next) => {
        onChange({ field, value: next });
      }}
    />
  );
  return (
    <fieldset disabled={isDisabled} className="m-0 flex flex-col gap-small rounded-field border-0 bg-well p-medium">
      <div className="flex items-center gap-small">
        {output.isVideo ? (
          <Film aria-hidden size={14} className="text-faint" />
        ) : (
          <Image aria-hidden size={14} className="text-faint" />
        )}
        <HoverHint text="Part of the file names">
          <SavedText
            label="Output name"
            placeholder={output.placeholder}
            value={output.name}
            onSave={(name) => {
              onChange({ field: "name", value: name });
            }}
          />
        </HoverHint>
        <span className="min-w-0 flex-1 truncate text-caption text-faint">{output.summary}</span>
        <Button variant="icon" icon={Trash2} title="Remove Output" hint="Remove output" onClick={onRemove} />
      </div>
      <div className="flex flex-wrap gap-small">
        {output.isVideo ? (
          <>
            {menu("Preset", output.videoPreset, options.videoPresets, "videoPreset")}
            <HoverHint text="Each video's subtitles, from its version">
              {menu("Subtitles", output.subtitles, options.subtitles, "subtitles")}
            </HoverHint>
          </>
        ) : (
          <>
            {menu("Format", output.format, options.formats, "format")}
            {menu("Size", output.size, options.sizes, "size")}
            {output.quality !== null && (
              <FilterMenu
                title="Quality"
                value={String(output.quality)}
                options={options.qualities}
                onChange={(next) => {
                  onChange({ field: "quality", value: Number(next) });
                }}
              />
            )}
            {menu("Color", output.colorSpace, options.colorSpaces, "colorSpace")}
            {output.background !== null && (
              <HoverHint text="Fill for transparent areas">
                <label className="flex items-center gap-small text-caption text-muted">
                  Background
                  <input
                    type="color"
                    aria-label="Background"
                    value={output.background}
                    onChange={(event) => {
                      onChange({ field: "background", value: event.target.value });
                    }}
                  />
                </label>
              </HoverHint>
            )}
          </>
        )}
        {menu("Crop", output.aspect, options.aspects, "aspect")}
      </div>
    </fieldset>
  );
}

function Footer({
  view,
  onNextRound,
  onShowFolder,
  onDelete,
  onExport,
  onCancel,
}: {
  view: DeliverySetView_Serialize;
  onNextRound: () => void;
  onShowFolder: () => void;
  onDelete: () => void;
  onExport: (scope: DeliveryExportScope) => void;
  onCancel: () => void;
}) {
  let actions: ReactNode;
  if (view.isExporting) {
    actions = (
      <>
        <Spinner />
        <Button variant="secondary" title="Cancel Export" onClick={onCancel} />
      </>
    );
  } else if (view.isFrozen) {
    actions = (
      <>
        {view.failedCount > 0 && (
          <Button
            variant="secondary"
            title={`Retry ${String(view.failedCount)} Failed`}
            onClick={() => {
              onExport("failed");
            }}
          />
        )}
        <Button
          variant="primary"
          title={view.pendingCount > 0 ? `Export ${String(view.pendingCount)} Pending` : "Exported"}
          hint="Export only changes"
          disabled={view.pendingCount === 0}
          onClick={() => {
            onExport("pending");
          }}
        />
      </>
    );
  } else {
    actions = (
      <Button
        variant="primary"
        title="Export…"
        disabled={!view.canExport}
        onClick={() => {
          onExport("all");
        }}
      />
    );
  }
  return (
    <div className="flex items-center gap-small border-t border-stroke p-large">
      <Button variant="secondary" icon={GitBranch} title="New Set from This One" hint="New set with newest versions" onClick={onNextRound} />
      {view.hasFolder && <Button variant="secondary" icon={Folder} title="Show in File Manager" onClick={onShowFolder} />}
      <Button variant="secondary" icon={Trash2} destructive title="Delete…" onClick={onDelete} />
      <span className="flex-1" />
      {actions}
    </div>
  );
}

/**
 * Moves one item's crop for one aspect (`DeliveryFrameSheet`): drag the frame or use the sliders. Rust works out the
 * crop, so what is set here is what the full-size export crops.
 */
function FrameSheet({
  id,
  versionId,
  aspect,
  onDone,
  onCancel,
}: {
  id: string;
  versionId: string;
  aspect: string;
  onDone: (frame: CropFrame) => void;
  onCancel: () => void;
}) {
  const view = useQuery({
    queryKey: [...deliveryKey, "frame", id, versionId, aspect],
    queryFn: () => commands.deliveryFrame(id, versionId, aspect),
  }).data;
  const [frame, setFrame] = useState<CropFrame | null>(null);
  const [loadedAspect, setLoadedAspect] = useState<number | null>(null);
  const shown = frame ?? view?.frame ?? null;
  const pictureAspect = view?.pictureAspect ?? loadedAspect;
  const rect = useQuery({
    queryKey: [...deliveryKey, "crop", aspect, pictureAspect, shown],
    queryFn: () => (shown === null || pictureAspect === null ? null : commands.deliveryCropRect(shown, aspect, pictureAspect)),
    enabled: shown !== null && pictureAspect !== null,
    placeholderData: (previous) => previous,
  }).data;
  const canvas = useRef<HTMLDivElement>(null);

  const move = (event: React.PointerEvent) => {
    const bounds = canvas.current?.getBoundingClientRect();
    if (!bounds || shown === null || event.buttons === 0) return;
    const clamp = (value: number) => Math.min(Math.max(value, 0), 1);
    setFrame({
      ...shown,
      centerX: clamp((event.clientX - bounds.left) / bounds.width),
      centerY: clamp((event.clientY - bounds.top) / bounds.height),
    });
  };
  const slider = (title: string, key: keyof CropFrame, min: number, max: number) => (
    <label className="flex items-center gap-medium">
      <span className="w-90 shrink-0 text-caption font-semibold text-muted">{title}</span>
      <input
        type="range"
        aria-label={title}
        min={min}
        max={max}
        step={0.01}
        value={shown?.[key] ?? min}
        onChange={(event) => {
          if (shown) setFrame({ ...shown, [key]: Number(event.target.value) });
        }}
        className="flex-1 accent-accent"
      />
    </label>
  );

  return (
    <Dialog title={view?.title ?? "Frame"} onCancel={onCancel} className="max-w-560">
      <div className="flex flex-col gap-4">
        <p className="m-0 text-heading font-bold">{view?.title}</p>
        <p className="m-0 truncate text-caption-plus text-muted">{view?.itemName}</p>
      </div>
      <div className="flex h-340 items-center justify-center rounded-small bg-well">
        {view?.preview.url ? (
          <div
            ref={canvas}
            className="relative max-h-full max-w-full touch-none overflow-hidden"
            onPointerDown={move}
            onPointerMove={move}
          >
            {view.isVideo ? (
              <video
                src={view.preview.url}
                muted
                preload="metadata"
                className="block max-h-340 max-w-full"
                onLoadedMetadata={(event) => {
                  const video = event.currentTarget;
                  if (video.videoHeight > 0) setLoadedAspect(video.videoWidth / video.videoHeight);
                }}
              />
            ) : (
              <img
                src={view.preview.url}
                alt=""
                draggable={false}
                className="block max-h-340 max-w-full"
                onLoad={(event) => {
                  const image = event.currentTarget;
                  if (image.naturalHeight > 0) setLoadedAspect(image.naturalWidth / image.naturalHeight);
                }}
              />
            )}
            {rect && (
              <div
                aria-hidden
                className="absolute border-2 border-accent shadow-[0_0_0_9999px_var(--color-scrim)]"
                style={{
                  left: `${String((rect.x ?? 0) * 100)}%`,
                  top: `${String((rect.y ?? 0) * 100)}%`,
                  width: `${String((rect.width ?? 0) * 100)}%`,
                  height: `${String((rect.height ?? 0) * 100)}%`,
                }}
              />
            )}
          </div>
        ) : (
          <Spinner />
        )}
      </div>
      {slider("Zoom", "zoom", 1, view?.maximumZoom ?? 4)}
      {slider("Across", "centerX", 0, 1)}
      {slider("Up and down", "centerY", 0, 1)}
      <div className="flex gap-small">
        <Button
          variant="secondary"
          title="Center"
          onClick={() => {
            if (view) setFrame(view.centered);
          }}
        />
        <span className="flex-1" />
        <Button variant="secondary" title="Cancel" onClick={onCancel} />
        <Button
          variant="primary"
          title="Done"
          disabled={shown === null}
          onClick={() => {
            if (shown) onDone(shown);
          }}
        />
      </div>
    </Dialog>
  );
}
