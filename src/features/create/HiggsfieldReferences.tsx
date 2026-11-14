import { useQuery } from "@tanstack/react-query";
import { Boxes, Clock, Images } from "lucide-react";
import { useEffect, useState } from "react";

import {
  commands,
  events,
  type AppError,
  type ElementsPanel_Serialize,
  type HiggsfieldReferencesView_Serialize,
  type HiggsfieldSource,
  type UploadsPanel_Serialize,
} from "../../bindings";
import { Dialog } from "../../app/Dialog";
import { EmptyState, MediaTag } from "../../design/Components";
import { cx } from "../../design/cx";
import { PagedFeedFooter } from "../../design/PagedFeedFooter";
import { FilterMenu, SegmentedControl } from "../../design/Pickers";
import { RemoteThumbnail } from "../../design/RemoteThumbnail";
import { Spinner } from "../../design/Spinner";
import { Button, focusRing } from "../../design/Styles";
import { useInvalidateOn } from "../../lib/events";
import { queryKeys } from "../../lib/queryKeys";
import { CharacterSheetsPicker } from "./CreateCharacter";
import { showError } from "./createData";

const key = queryKeys.create.higgsfield("references");

/** The sheet's contents; `CreateChanged` asks again as pages arrive and downloads end. */
function useHiggsfieldReferences() {
  useInvalidateOn(events.createChanged, [key]);
  return useQuery<HiggsfieldReferencesView_Serialize | null, AppError>({
    queryKey: key,
    queryFn: () => commands.createHiggsfieldReferences(),
  });
}

function load(source: HiggsfieldSource, more: boolean) {
  commands.createLoadHiggsfield(source, more).catch((error: unknown) => {
    showError("Can't load from Higgsfield", error);
  });
}

function loadCharacters(more: boolean) {
  load("characters", more);
}

function addTitle(count: number) {
  return count > 1 ? `Add ${String(count)} Images` : "Add Image";
}

/**
 * References › From Higgsfield (`HiggsfieldReferencesSheet` in `UploadsPicker.swift`): the account's uploaded images
 * and, on an account, its elements. Picked images are downloaded once and added as references; closing the sheet
 * drops a download still running.
 */
export function HiggsfieldReferences({ onClose }: { onClose: () => void }) {
  const view = useHiggsfieldReferences().data ?? null;
  const [source, setSource] = useState<HiggsfieldSource>("uploads");
  const close = () => {
    void commands.createCloseHiggsfield();
    onClose();
  };
  return (
    <Dialog title="From Higgsfield" onCancel={close} className="h-560 max-w-640">
      {view !== null && view.sources.length > 1 && (
        <div className="self-start">
          <SegmentedControl aria-label="Source" value={source} options={view.sources} onChange={setSource} />
        </div>
      )}
      {view === null ? (
        <div className="flex flex-1 items-center justify-center">
          <Spinner />
        </div>
      ) : source === "characters" && view.characters !== null ? (
        <CharacterSheetsPicker
          panel={view.characters}
          adding={view.adding}
          onLoad={loadCharacters}
          onClose={close}
          onDone={onClose}
        />
      ) : source === "elements" && view.elements !== null ? (
        <ElementsPicker
          panel={view.elements}
          limit={view.limit}
          adding={view.adding}
          onClose={close}
          onDone={onClose}
        />
      ) : (
        <UploadsPicker
          panel={view.uploads}
          limit={view.limit}
          adding={view.adding}
          onClose={close}
          onDone={onClose}
        />
      )}
    </Dialog>
  );
}

function Header({ title, detail, trailing }: { title: string; detail: string; trailing?: React.ReactNode }) {
  return (
    <div className="flex items-start gap-small">
      <div className="flex flex-1 flex-col gap-4">
        <p className="m-0 text-heading font-bold text-text">{title}</p>
        <p className="m-0 text-body text-muted">{detail}</p>
      </div>
      {trailing}
    </div>
  );
}

function Tile({
  url,
  label,
  number,
  isDisabled,
  onClick,
  children,
}: {
  url: string | null;
  label: string;
  number?: number | undefined;
  isDisabled: boolean;
  onClick: () => void;
  children?: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      disabled={isDisabled}
      className={cx(
        "relative block aspect-square w-full overflow-hidden rounded-small border bg-well enabled:cursor-pointer disabled:opacity-60",
        number === undefined ? "border-stroke" : "border-2 border-accent",
        focusRing,
      )}
      onClick={onClick}
    >
      <RemoteThumbnail url={url} className="absolute inset-0 size-full" />
      {number !== undefined && (
        <span className="absolute top-6 left-6 flex size-20 items-center justify-center rounded-full bg-accent text-micro font-heavy text-ink">
          {number}
        </span>
      )}
      {children}
    </button>
  );
}

function Actions({
  failure,
  isBusy,
  children,
}: {
  failure: string | null;
  isBusy: boolean;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-small">
      {failure !== null && <p className="m-0 line-clamp-2 text-small font-medium text-muted">{failure}</p>}
      <span className="flex-1" />
      {isBusy && <Spinner />}
      {children}
    </div>
  );
}

/** Picks, numbered in the order made, up to `limit`. */
function usePicks<Value>(limit: number, initial: Value[] = []) {
  const [picked, setPicked] = useState<Value[]>(initial);
  const toggle = (value: Value) => {
    setPicked((old) =>
      old.includes(value) ? old.filter((item) => item !== value) : old.length < limit ? [...old, value] : old,
    );
  };
  return { picked, setPicked, toggle };
}

function UploadsPicker({
  panel,
  limit,
  adding,
  onClose,
  onDone,
}: {
  panel: UploadsPanel_Serialize;
  limit: number;
  adding: string | null;
  onClose: () => void;
  onDone: () => void;
}) {
  const { picked, toggle } = usePicks<string>(limit);
  const [failure, setFailure] = useState<string | null>(null);
  const isAdding = adding !== null;
  useEffect(() => {
    load("uploads", false);
  }, []);
  const add = () => {
    setFailure(null);
    commands.createAddUploads(picked).then(
      (message) => {
        if (message === null) onDone();
        else setFailure(message);
      },
      (error: unknown) => {
        showError("Can't download the images", error);
      },
    );
  };
  return (
    <>
      <Header title={panel.title} detail={panel.detail} />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="grid grid-cols-[repeat(auto-fill,minmax(104px,1fr))] gap-small">
          {panel.tiles.map((tile) => {
            const index = picked.indexOf(tile.id);
            const number = index >= 0 ? index + 1 : undefined;
            return (
              <Tile
                key={tile.id}
                url={tile.url}
                label={number === undefined ? "Uploaded image" : `Uploaded image, picked ${String(number)}`}
                number={number}
                isDisabled={isAdding || (number === undefined && picked.length >= limit)}
                onClick={() => {
                  toggle(tile.id);
                }}
              />
            );
          })}
        </div>
        {panel.failure !== null || panel.hasMore ? (
          <PagedFeedFooter
            failure={panel.failure ?? undefined}
            hasMore={panel.hasMore}
            count={panel.tiles.length}
            loadMore={() => {
              load("uploads", true);
            }}
          />
        ) : (
          panel.isEmpty && (
            <EmptyState
              icon={Images}
              title="No uploaded images yet"
              detail="Reference images sent from this app or higgsfield.ai appear here."
              className="pt-60"
            />
          )
        )}
      </div>
      <Actions failure={failure} isBusy={isAdding}>
        <Button variant="secondary" title="Cancel" onClick={onClose} />
        <Button
          variant="primary"
          title={addTitle(picked.length)}
          disabled={picked.length === 0 || isAdding}
          onClick={add}
        />
      </Actions>
    </>
  );
}

type Choosing = { name: string; detail: string; images: string[]; limit: number };

function ElementsPicker({
  panel,
  limit,
  adding,
  onClose,
  onDone,
}: {
  panel: ElementsPanel_Serialize;
  limit: number;
  adding: string | null;
  onClose: () => void;
  onDone: () => void;
}) {
  const [choosing, setChoosing] = useState<Choosing | null>(null);
  const picks = usePicks<number>(choosing?.limit ?? limit);
  const [failure, setFailure] = useState<string | null>(null);
  const isAvailable = panel.unavailableReason === null;
  useEffect(() => {
    if (isAvailable) load("elements", false);
  }, [isAvailable]);

  const answer = (promise: ReturnType<typeof commands.createOpenElement>) => {
    setFailure(null);
    promise.then(
      (opening) => {
        if (opening.kind === "added") onDone();
        else if (opening.kind === "choose") {
          picks.setPicked(opening.picked);
          setChoosing(opening);
        } else if (opening.message !== "") setFailure(opening.message);
      },
      (error: unknown) => {
        showError("Can't open this element", error);
      },
    );
  };

  return (
    <>
      <Header
        title={choosing?.name ?? "Elements"}
        detail={choosing?.detail ?? panel.detail}
        trailing={
          choosing === null &&
          isAvailable && (
            <FilterMenu
              title="Category"
              value={panel.category}
              options={panel.categories}
              onChange={(category) => {
                commands.createSetElementCategory(category).catch((error: unknown) => {
                  showError("Can't filter the elements", error);
                });
              }}
            />
          )
        }
      />
      {panel.unavailableReason !== null ? (
        <EmptyState icon={Clock} title={panel.unavailableReason} className="flex-1 justify-center" />
      ) : choosing !== null ? (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="grid grid-cols-[repeat(auto-fill,minmax(104px,1fr))] gap-small">
            {choosing.images.map((url, index) => {
              const position = picks.picked.indexOf(index);
              const number = position >= 0 ? position + 1 : undefined;
              return (
                <Tile
                  key={url}
                  url={url}
                  label={number === undefined ? "Element image" : `Element image, picked ${String(number)}`}
                  number={number}
                  isDisabled={
                    adding !== null || (number === undefined && picks.picked.length >= choosing.limit)
                  }
                  onClick={() => {
                    picks.toggle(index);
                  }}
                />
              );
            })}
          </div>
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="grid grid-cols-[repeat(auto-fill,minmax(120px,1fr))] gap-small">
            {panel.tiles.map((element) => (
              <div key={element.id} className="flex flex-col gap-4">
                <Tile
                  url={element.thumbnailUrl}
                  label={element.name}
                  isDisabled={adding !== null || limit <= 0}
                  onClick={() => {
                    answer(commands.createOpenElement(element.id));
                  }}
                >
                  {adding === element.id && (
                    <span className="absolute inset-0 flex items-center justify-center">
                      <Spinner />
                    </span>
                  )}
                  {element.tag !== null && (
                    <span className="absolute bottom-4 left-4">
                      <MediaTag text={element.tag} />
                    </span>
                  )}
                </Tile>
                <span className="truncate text-small font-semibold text-text">{element.name}</span>
              </div>
            ))}
          </div>
          {panel.failure !== null || panel.hasMore ? (
            <PagedFeedFooter
              failure={panel.failure ?? undefined}
              hasMore={panel.hasMore}
              count={panel.tiles.length}
              loadMore={() => {
                load("elements", true);
              }}
            />
          ) : (
            panel.isEmpty && (
              <EmptyState
                icon={Boxes}
                title="No elements yet"
                detail="Characters, environments and props saved on higgsfield.ai appear here."
                className="pt-60"
              />
            )
          )}
        </div>
      )}
      <Actions failure={failure} isBusy={adding !== null}>
        {choosing !== null ? (
          <>
            <Button
              variant="secondary"
              title="Back"
              onClick={() => {
                void commands.createCloseElement();
                setChoosing(null);
                picks.setPicked([]);
              }}
            />
            <Button
              variant="primary"
              title={addTitle(picks.picked.length)}
              disabled={picks.picked.length === 0 || adding !== null}
              onClick={() => {
                answer(commands.createAddElementImages(picks.picked));
              }}
            />
          </>
        ) : (
          <Button variant="secondary" title="Cancel" onClick={onClose} />
        )}
      </Actions>
    </>
  );
}
