import { CheckCircle2, Ellipsis, UserSquare2, XCircle } from "lucide-react";
import { useState } from "react";

import {
  commands,
  type CharacterSetsView_Serialize,
  type CharacterSetView_Serialize,
  type CreateEdit_Deserialize,
  type ImageView_Serialize,
  type UserMessage_Serialize,
} from "../../bindings";
import { AlertDialog } from "../../app/Dialog";
import { DropZone, SectionLabel } from "../../design/Components";
import { cx } from "../../design/cx";
import { HoverHint } from "../../design/HoverHint";
import { Spinner } from "../../design/Spinner";
import { Button, focusRing, TextField } from "../../design/Styles";
import { ContextMenu } from "../library/LibraryDialogs";
import { showError, useCreateNotice, useMediaUrl } from "./createData";

/** What the save dialog and its failure need. */
const saveTitle = "Save to Higgsfield…";
const savedTitle = "Saved to Higgsfield";

/** Where a file dropped on the window lands: the closest element with this attribute names it. */
export const characterDropTarget = "characters";

function addedNotice(notice: { title: string; detail: string } | null) {
  if (notice !== null) useCreateNotice.getState().show(notice);
}

function chooseCharacterSet() {
  commands.createChooseCharacterSet().then(addedNotice, (error: unknown) => {
    showError("Can't add the character", error);
  });
}

/** Images dropped on the characters: one new batch character. */
export function addCharacterSet(paths: string[]) {
  commands.createAddCharacterSet(paths).then(addedNotice, (error: unknown) => {
    showError("Can't add the character", error);
  });
}

/**
 * A motion transfer or object swap batch (`CharacterSetsSection` in `CreateInputs.swift`): one row per character,
 * each saved to Higgsfield as an element or removed, then Add a character. Shown instead of the frames and references.
 */
export function CharacterSets({
  sets,
  edit,
}: {
  sets: CharacterSetsView_Serialize;
  edit: (edit: CreateEdit_Deserialize) => void;
}) {
  const [naming, setNaming] = useState<{ index: number; name: string } | null>(null);
  const [failure, setFailure] = useState<UserMessage_Serialize | null>(null);

  const save = (index: number, name: string) => {
    commands.createSaveCharacterSet(index, name).then(setFailure, (error: unknown) => {
      showError("Can't save to Higgsfield", error);
    });
  };
  const named = naming === null ? undefined : sets.sets[naming.index];

  return (
    <section className="flex flex-col gap-small" data-drop={characterDropTarget}>
      <SectionLabel title="Characters" detail={sets.detail} />
      {sets.sets.map((set, index) => (
        <CharacterRow
          key={`${String(index)}-${set.imageCount.toString()}`}
          index={index}
          set={set}
          onSave={() => {
            setNaming({ index, name: `Character ${String(index + 1)}` });
          }}
          onRemove={() => {
            edit({ kind: "removeCharacterSet", index });
          }}
        />
      ))}
      {sets.notice !== null ? (
        <p className="m-0 text-caption text-warning">{sets.notice}</p>
      ) : (
        <DropZone>
          <button
            type="button"
            className={cx(
              "relative flex w-full cursor-pointer flex-col items-center gap-4 p-medium text-center",
              focusRing,
            )}
            onClick={chooseCharacterSet}
          >
            <span className="flex items-center gap-6 text-body font-semibold text-accent">
              <UserSquare2 aria-hidden size={14} /> Add a character
            </span>
            <span className="text-small text-muted">1–8 images of one character</span>
          </button>
        </DropZone>
      )}
      {naming !== null && named !== undefined && (
        <AlertDialog
          title={saveTitle}
          message={named.saveExplanation}
          onCancel={() => {
            setNaming(null);
          }}
        >
          <form
            className="flex w-full flex-col gap-small"
            onSubmit={(event) => {
              event.preventDefault();
              save(naming.index, naming.name);
              setNaming(null);
            }}
          >
            <TextField
              aria-label="Name"
              placeholder="Name"
              data-autofocus
              value={naming.name}
              onChange={(event) => {
                setNaming({ index: naming.index, name: event.target.value });
              }}
            />
            <div className="flex justify-end gap-small">
              <Button
                variant="secondary"
                title="Cancel"
                onClick={() => {
                  setNaming(null);
                }}
              />
              <Button variant="primary" type="submit" title="Save" />
            </div>
          </form>
        </AlertDialog>
      )}
      {failure !== null && (
        <AlertDialog
          title={failure.title}
          message={failure.message}
          onCancel={() => {
            setFailure(null);
          }}
        >
          <Button
            variant="primary"
            title="OK"
            onClick={() => {
              setFailure(null);
            }}
          />
        </AlertDialog>
      )}
    </section>
  );
}

function Thumb({ image }: { image: ImageView_Serialize }) {
  const url = useMediaUrl(image.thumbnail);
  return url ? (
    <img src={url} alt="" draggable={false} className="size-40 rounded-small object-cover" />
  ) : (
    <span className="size-40 rounded-small bg-glass" />
  );
}

function CharacterRow({
  index,
  set,
  onSave,
  onRemove,
}: {
  index: number;
  set: CharacterSetView_Serialize;
  onSave: () => void;
  onRemove: () => void;
}) {
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const number = String(index + 1);
  return (
    <div className="flex items-center gap-small rounded-field border border-stroke bg-well p-small">
      <div className="flex w-92 shrink-0 flex-col gap-2">
        <span className="text-body font-semibold text-text">{set.title}</span>
        <span className="text-caption text-faint">{set.detail}</span>
        {set.save === "saved" && (
          <span className="flex items-center gap-4 truncate text-caption text-accent">
            <CheckCircle2 aria-hidden size={11} /> {savedTitle}
          </span>
        )}
      </div>
      <div className="flex gap-4">
        {set.images.map((image) => (
          <Thumb key={image.path} image={image} />
        ))}
      </div>
      <span className="flex-1" />
      {set.save === "saving" && <Spinner label="Saving to Higgsfield" />}
      {set.save === "available" && (
        <HoverHint text="More">
          <button
            type="button"
            aria-label={`More for character ${number}`}
            className={cx("flex cursor-pointer text-accent", focusRing)}
            onClick={(event) => {
              const box = event.currentTarget.getBoundingClientRect();
              setMenu({ x: box.left, y: box.bottom + 4 });
            }}
          >
            <Ellipsis aria-hidden size={16} />
          </button>
        </HoverHint>
      )}
      {menu !== null && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={[{ title: saveTitle, run: onSave }]}
          onClose={() => {
            setMenu(null);
          }}
        />
      )}
      <HoverHint text="Remove character">
        <button
          type="button"
          aria-label={`Remove character ${number}`}
          className={cx("flex cursor-pointer text-accent", focusRing)}
          onClick={onRemove}
        >
          <XCircle aria-hidden size={16} />
        </button>
      </HoverHint>
    </div>
  );
}
