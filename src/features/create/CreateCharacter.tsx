import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronRight, Dices, TriangleAlert, UserRoundPlus, UserSquare } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import {
  commands,
  events,
  type AppError,
  type CharacterEdit_Deserialize,
  type CharacterPresetsView_Serialize,
  type CharacterSnapshot_Serialize,
  type CharactersPanel_Serialize,
  type TraitGroupView_Serialize,
} from "../../bindings";
import { EmptyState, SectionLabel } from "../../design/Components";
import { cx } from "../../design/cx";
import { FlowLayout } from "../../design/FlowLayout";
import { HoverHint } from "../../design/HoverHint";
import { PagedFeedFooter } from "../../design/PagedFeedFooter";
import { FilterMenu, SegmentedControl } from "../../design/Pickers";
import { RemoteThumbnail } from "../../design/RemoteThumbnail";
import { Spinner } from "../../design/Spinner";
import { Button, focusRing, Well } from "../../design/Styles";
import { useInvalidateOn } from "../../lib/events";
import { queryKeys } from "../../lib/queryKeys";
import { showError, useCreateNotice } from "./createData";
import {
  GenerateButton,
  GenerateSheetLayout,
  PendingTile,
  RemovableThumb,
  SheetOptionRow,
  SquareThumb,
} from "./GenerateSheet";
import { useImagesSnapshot } from "./imageData";
import { UnsavedRecordsWarning } from "./ImageSide";
import { RequestRow } from "./RequestRow";

/** Create Character (`characters_snapshot`), computed in Rust; `ImagesChanged` asks for it again. */
function useCharacterSnapshot() {
  useInvalidateOn(events.imagesChanged, [queryKeys.images.all]);
  return useQuery<CharacterSnapshot_Serialize, AppError>({
    queryKey: queryKeys.images.characters,
    queryFn: () => commands.charactersSnapshot(),
  });
}

type CharacterEdit = (edit: CharacterEdit_Deserialize) => void;

function useCharacterEdit(): CharacterEdit {
  const client = useQueryClient();
  return useCallback(
    (edit) => {
      commands.charactersEdit(edit).then(
        (snapshot) => {
          client.setQueryData(queryKeys.images.characters, snapshot);
        },
        (error: unknown) => {
          showError("Can't change the form", error);
        },
      );
    },
    [client],
  );
}

/** Generate; asks first when the price takes today past the cap, then generates again approved. */
async function generateCharacters(approved = false): Promise<void> {
  try {
    const outcome = await commands.charactersGenerate(approved);
    if (outcome.kind === "confirm") {
      useCreateNotice.getState().ask(outcome.confirmation, () => void generateCharacters(true));
    } else if (outcome.kind === "failed") {
      useCreateNotice.getState().show(outcome.notice);
    }
  } catch (error) {
    showError("Can't create the character", error);
  }
}

/** The paths of files dropped on a row. */
function droppedPaths(event: React.DragEvent): string[] {
  return Array.from(event.dataTransfer.files)
    .map((file) => (file as File & { path?: string }).path)
    .filter((path): path is string => typeof path === "string" && path !== "");
}

/**
 * Create Character (`CreateCharacterSheet`): designs a person or an animal and makes character sheets of it on the
 * account. Laid out like Generate reference images: the form on the left, the requests sent from here on the right,
 * and Generate with Higgsfield's quote of the exact request under them. Finished sheets are saved to Library › Images
 * and listed in References › From Higgsfield › Characters, so the sheet can close while they're being made.
 */
export function CreateCharacterSheet({ onClose }: { onClose: () => void }) {
  const snapshot = useCharacterSnapshot().data;
  const unsaved = useImagesSnapshot().data?.unsaved ?? null;
  const edit = useCharacterEdit();
  const isBusy = snapshot?.generate.isBusy ?? false;
  return (
    <GenerateSheetLayout
      title="Create Character"
      subtitle="A character sheet shows one person or animal from every side. Use it as a reference to keep them the same in every video."
      formWidth="w-400"
      isBusy={isBusy}
      closeHint={snapshot?.closeHint ?? "Close"}
      onClose={onClose}
      form={snapshot ? <CharacterForm snapshot={snapshot} edit={edit} /> : <Spinner />}
      results={
        <div className="flex min-h-0 flex-1 flex-col gap-small">
          <SectionLabel title="Requests" />
          {unsaved && <UnsavedRecordsWarning unsaved={unsaved} />}
          <div className="flex min-h-0 flex-1 flex-col gap-small overflow-y-auto">
            {snapshot?.runs.length === 0 && (
              <p className="m-0 flex min-h-240 items-center justify-center text-center text-body text-faint">
                Each sheet takes a few minutes. You can close this window: the sheets keep going and show in
                Characters when they're done.
              </p>
            )}
            {snapshot?.runs.map((run) => (
              <RequestRow key={run.id} row={run} onCancel={() => undefined} />
            ))}
          </div>
        </div>
      }
      actions={
        snapshot && (
          <>
            {snapshot.notice && (
              <p
                className={cx(
                  "m-0",
                  snapshot.notice.isQuiet ? "line-clamp-2 text-caption text-faint" : "text-body text-muted",
                )}
              >
                {snapshot.notice.text}
              </p>
            )}
            <span className="flex-1" />
            <GenerateButton
              isGenerating={isBusy}
              price={snapshot.generate.price}
              hint={snapshot.generate.hint}
              disabled={!snapshot.generate.enabled}
              onClick={() => void generateCharacters()}
            />
          </>
        )
      }
    />
  );
}

/**
 * Create Character's form, top to bottom (`CreateCharacterForm`): presets, a person or an animal and its look, a
 * description, then the face, style references, Randomize and the number of sheets as compact rows, and last the
 * tier's traits, one collapsed row per group that opens to its chips.
 */
function CharacterForm({ snapshot, edit }: { snapshot: CharacterSnapshot_Serialize; edit: CharacterEdit }) {
  const client = useQueryClient();
  const choose = (styles: boolean) => {
    commands.charactersChooseImages(styles).then(
      (next) => {
        client.setQueryData(queryKeys.images.characters, next);
      },
      (error: unknown) => {
        showError("Can't add the image", error);
      },
    );
  };
  return (
    <div className="flex flex-col gap-18">
      {snapshot.presets && <Presets presets={snapshot.presets} edit={edit} />}
      <section className="flex flex-col gap-small">
        <SectionLabel title="Character" />
        <div className="flex items-center gap-small">
          <SegmentedControl
            aria-label="Character"
            value={snapshot.kind}
            options={snapshot.kinds}
            onChange={(value) => {
              edit({ kind: "selectKind", value });
            }}
          />
          {snapshot.tiers.length > 0 && (
            <FilterMenu
              title="Look"
              value={snapshot.tier}
              options={snapshot.tiers}
              onChange={(value) => {
                edit({ kind: "selectTier", value });
              }}
            />
          )}
        </div>
      </section>
      <Description snapshot={snapshot} edit={edit} />
      <Well className="px-medium">
        <SheetOptionRow
          title="Face"
          detail="Optional · a photo of the face to keep"
          drop={(paths) => {
            edit({ kind: "dropFace", paths });
          }}
        >
          <div
            className="contents"
            onDrop={(event) => {
              event.preventDefault();
              edit({ kind: "dropFace", paths: droppedPaths(event) });
            }}
          >
            {snapshot.face !== null ? (
              <RemovableThumb
                path={snapshot.face}
                onRemove={() => {
                  edit({ kind: "setFace", path: null });
                }}
              />
            ) : (
              <Button
                variant="secondary"
                title="Choose…"
                aria-label="Choose a face"
                onClick={() => {
                  choose(false);
                }}
              />
            )}
          </div>
        </SheetOptionRow>
        <SheetOptionRow title="Style" detail="Optional · up to 3 images of the look to follow">
          <div
            className="flex items-center gap-6"
            onDragOver={(event) => {
              event.preventDefault();
            }}
            onDrop={(event) => {
              event.preventDefault();
              edit({ kind: "addStyles", paths: droppedPaths(event) });
            }}
          >
            {snapshot.styles.map((path, index) => (
              <RemovableThumb
                key={`${path}-${String(index)}`}
                path={path}
                onRemove={() => {
                  edit({ kind: "removeStyle", index });
                }}
              />
            ))}
            {snapshot.canAddStyle && (
              <Button
                variant="secondary"
                title="Choose…"
                aria-label="Choose style images"
                onClick={() => {
                  choose(true);
                }}
              />
            )}
          </div>
        </SheetOptionRow>
        <SheetOptionRow title="Randomize" detail="Fills in what the form leaves open differently each time">
          <input
            type="checkbox"
            role="switch"
            aria-label="Randomize"
            checked={snapshot.randomize}
            className="size-18 cursor-pointer accent-accent"
            onChange={(event) => {
              edit({ kind: "setRandomize", on: event.target.checked });
            }}
          />
        </SheetOptionRow>
        <SheetOptionRow title="Sheets">
          <SegmentedControl
            isCompact
            aria-label="Sheets"
            value={snapshot.count}
            options={snapshot.counts.map((count) => ({ value: count, title: String(count) }))}
            onChange={(count) => {
              edit({ kind: "setCount", count });
            }}
          />
        </SheetOptionRow>
      </Well>
      {snapshot.traits && (
        <section className="flex flex-col gap-small">
          <SectionLabel title="Traits" detail={snapshot.traits.detail ?? undefined} />
          {snapshot.traits.failure !== null && (
            <p className="m-0 flex items-center gap-6 text-caption text-muted">
              <TriangleAlert aria-hidden size={12} />
              {snapshot.traits.failure}
            </p>
          )}
          {snapshot.traits.groups.length > 0 && <Traits groups={snapshot.traits.groups} edit={edit} />}
        </section>
      )}
    </div>
  );
}

/** Ready-made characters: a click fills the form with one. */
function Presets({ presets, edit }: { presets: CharacterPresetsView_Serialize; edit: CharacterEdit }) {
  const count = presets.tiles.length;
  useEffect(() => {
    if (presets.hasMore && presets.failure === null) {
      commands.charactersLoadMorePresets().catch((error: unknown) => {
        showError("Can't load the presets", error);
      });
    }
  }, [count, presets.hasMore, presets.failure]);
  return (
    <section className="flex flex-col gap-small">
      <div className="flex items-center gap-small">
        <SectionLabel title="Presets" />
        {presets.showsClear && (
          <Button
            variant="textLink"
            title="Clear Preset"
            onClick={() => {
              edit({ kind: "clearPreset" });
            }}
          />
        )}
      </div>
      <div className="flex gap-small overflow-x-auto">
        {presets.tiles.map((preset) => (
          <HoverHint key={preset.id} text={preset.hint} className="shrink-0">
            <button
              type="button"
              aria-label={preset.name}
              aria-pressed={preset.isPicked}
              disabled={!preset.enabled}
              className={cx(
                "flex w-84 flex-col gap-4 text-left enabled:cursor-pointer disabled:opacity-40",
                focusRing,
              )}
              onClick={() => {
                edit({ kind: "applyPreset", id: preset.id });
              }}
            >
              <span
                className={cx(
                  "relative block size-84 overflow-hidden rounded-small bg-well",
                  preset.isPicked ? "border-2 border-accent" : "border border-stroke",
                )}
              >
                <RemoteThumbnail url={preset.previewUrl} className="absolute inset-0 size-full" />
              </span>
              <span
                className={cx(
                  "truncate text-micro font-semibold",
                  preset.isPicked ? "text-accent" : "text-muted",
                )}
              >
                {preset.name}
              </span>
            </button>
          </HoverHint>
        ))}
        {presets.hasMore && presets.failure === null && (
          <span className="flex w-40 shrink-0 items-center justify-center">
            <Spinner />
          </span>
        )}
      </div>
    </section>
  );
}

function Description({ snapshot, edit }: { snapshot: CharacterSnapshot_Serialize; edit: CharacterEdit }) {
  const field = useRef<HTMLTextAreaElement>(null);
  const [text, setText] = useState(snapshot.brief);
  useEffect(() => {
    if (document.activeElement !== field.current) setText(snapshot.brief);
  }, [snapshot.brief]);
  return (
    <section className="flex flex-col gap-small">
      <SectionLabel title="Description" detail={snapshot.briefDetail ?? undefined} />
      <textarea
        ref={field}
        aria-label="Description"
        placeholder="Who they are, how they look and dress"
        value={text}
        className="min-h-90 max-h-160 w-full resize-none rounded-field border border-stroke bg-well p-small text-body text-text outline-none placeholder:text-faint focus:border-accent"
        onChange={(event) => {
          setText(event.target.value);
          edit({ kind: "setBrief", text: event.target.value });
        }}
      />
    </section>
  );
}

/** The tier's trait groups, each a collapsed row with its choice; a click opens its chips under it, one at a time. */
function Traits({ groups, edit }: { groups: TraitGroupView_Serialize[]; edit: CharacterEdit }) {
  const [open, setOpen] = useState<string | null>(null);
  return (
    <Well className="px-medium">
      {groups.map((group) => {
        const isOpen = open === group.id;
        return (
          <div key={group.id} className="border-b border-stroke last:border-b-0">
            <button
              type="button"
              aria-label={group.title}
              aria-expanded={isOpen}
              aria-description={group.accessibilityValue}
              title={isOpen ? "Hides the choices" : "Shows the choices"}
              className={cx(
                "flex min-h-36 w-full cursor-pointer items-center gap-small text-left",
                focusRing,
              )}
              onClick={() => {
                setOpen(isOpen ? null : group.id);
              }}
            >
              <span className="truncate text-body text-text">{group.title}</span>
              <span className="flex-1" />
              {group.choiceKind === "picked" ? (
                <span className="truncate text-small font-semibold text-text">{group.choice}</span>
              ) : group.choiceKind === "random" ? (
                <span className="flex items-center gap-4 text-small text-muted">
                  <Dices aria-hidden size={12} />
                  {group.choice}
                </span>
              ) : (
                <span className="text-small text-faint">{group.choice}</span>
              )}
              {group.limit !== null && (
                <span className="shrink-0 text-caption text-faint tabular-nums">{group.limit}</span>
              )}
              <ChevronRight
                aria-hidden
                size={10}
                strokeWidth={3}
                className={cx("text-faint transition-transform", isOpen && "rotate-90")}
              />
            </button>
            {isOpen && (
              <div className="pb-medium">
                <FlowLayout>
                  {group.traits.map((item) => (
                    <button
                      key={item.id}
                      type="button"
                      aria-pressed={item.isPicked}
                      className={cx(
                        "h-26 cursor-pointer rounded-full px-10 text-small",
                        item.isPicked
                          ? "bg-accent font-bold text-ink"
                          : "border border-stroke bg-well font-semibold text-muted hover:bg-glass-hover hover:text-text",
                        focusRing,
                      )}
                      onClick={() => {
                        edit({ kind: "toggleTrait", group: group.id, id: item.id });
                      }}
                    >
                      {item.title}
                    </button>
                  ))}
                </FlowLayout>
              </div>
            )}
          </div>
        );
      })}
    </Well>
  );
}

/**
 * References › From Higgsfield › Characters (`CharacterSheetsPicker`): the account's character sheets as a grid, with
 * Create Character… on top. Picking a sheet adds it as one reference image; sheets still being made show as
 * placeholders and appear here when they finish.
 */
export function CharacterSheetsPicker({
  panel,
  adding,
  onLoad,
  onClose,
  onDone,
}: {
  panel: CharactersPanel_Serialize;
  adding: string | null;
  onLoad: (more: boolean) => void;
  onClose: () => void;
  onDone: () => void;
}) {
  const [isCreating, setIsCreating] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  // Sheets made here arrive with the image side's changes.
  useInvalidateOn(events.imagesChanged, [queryKeys.create.higgsfield("references")]);
  useEffect(() => {
    onLoad(false);
  }, [onLoad]);
  const add = (id: string) => {
    setFailure(null);
    commands.createAddCharacter(id).then(
      (message) => {
        if (message === null) onDone();
        else setFailure(message);
      },
      (error: unknown) => {
        showError("Can't download the sheet", error);
      },
    );
  };
  return (
    <>
      <div className="flex items-start gap-small">
        <div className="flex flex-1 flex-col gap-4">
          <p className="m-0 text-heading font-bold text-text">{panel.title}</p>
          <p className="m-0 text-body text-muted">{panel.detail}</p>
        </div>
        <Button
          variant="primary"
          icon={UserRoundPlus}
          title="Create Character…"
          onClick={() => {
            setIsCreating(true);
          }}
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="grid grid-cols-[repeat(auto-fill,minmax(120px,1fr))] gap-small">
          {Array.from({ length: panel.pending }, (_, index) => (
            <PendingTile key={`pending-${String(index)}`} label="Character sheet being made" />
          ))}
          {panel.tiles.map((tile) => (
            <HoverHint key={tile.id} text={panel.fullHint ?? ""}>
              <button
                type="button"
                aria-label="Character sheet"
                disabled={adding !== null || panel.fullHint !== null}
                className={cx(
                  "relative block aspect-square w-full overflow-hidden rounded-small border border-stroke bg-well enabled:cursor-pointer disabled:opacity-60",
                  focusRing,
                )}
                onClick={() => {
                  add(tile.id);
                }}
              >
                <SquareThumb path={tile.path} url={tile.url} />
                {adding === tile.id && (
                  <span className="absolute inset-0 flex items-center justify-center">
                    <Spinner />
                  </span>
                )}
              </button>
            </HoverHint>
          ))}
        </div>
        {panel.listsAccount ? (
          (panel.failure !== null || panel.hasMore) && (
            <PagedFeedFooter
              failure={panel.failure ?? undefined}
              hasMore={panel.hasMore}
              count={panel.tiles.length}
              loadMore={() => {
                onLoad(true);
              }}
            />
          )
        ) : (
          <p className="m-0 pt-small text-caption text-faint">{panel.note}</p>
        )}
        {panel.isEmpty && (
          <EmptyState
            icon={UserSquare}
            title="No characters yet"
            detail="Create a character: a sheet of one person or animal from every side."
            className="pt-40"
          />
        )}
      </div>
      <div className="flex items-center gap-small">
        {failure !== null && <p className="m-0 line-clamp-2 text-small font-medium text-muted">{failure}</p>}
        <span className="flex-1" />
        {adding !== null && <Spinner />}
        <Button variant="secondary" title="Cancel" onClick={onClose} />
      </div>
      {isCreating && (
        <CreateCharacterSheet
          onClose={() => {
            setIsCreating(false);
          }}
        />
      )}
    </>
  );
}
