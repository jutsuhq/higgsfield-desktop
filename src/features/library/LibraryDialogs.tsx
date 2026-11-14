import { useNavigate } from "@tanstack/react-router";
import { X } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";

import { commands, type LinkView_Serialize } from "../../bindings";
import { AlertDialog, Dialog } from "../../app/Dialog";
import { SectionLabel } from "../../design/Components";
import { cx } from "../../design/cx";
import { Button, focusRing } from "../../design/Styles";
import { useImageDetails } from "./libraryData";

/** A menu at the pointer, as a card's context menu. Closes on a pick, Esc or a click elsewhere. */
export interface MenuItem {
  title: string;
  isDestructive?: boolean;
  isDisabled?: boolean;
  /** The tooltip. */
  hint?: string;
  /** A toggle: drawn with a checkmark when on. */
  isChecked?: boolean;
  run: () => void;
  /** A submenu, as Compare with… on a result's More menu: it opens beside the item, and `run` isn't called. */
  children?: (MenuItem | "divider")[];
}

/** The items of a menu or of a submenu; a pick closes the whole menu. */
function MenuList({
  items,
  onClose,
  className,
  style,
}: {
  items: (MenuItem | "divider")[];
  onClose: () => void;
  className: string;
  style?: { left?: number; top?: number };
}) {
  const [open, setOpen] = useState<number | null>(null);
  return (
    <div
      role="menu"
      style={style}
      className={cx(
        "flex min-w-180 flex-col rounded-field border border-glass-stroke bg-background p-x-small shadow-toast",
        className,
      )}
    >
      {items.map((item, index) =>
        item === "divider" ? (
          <hr key={`divider-${String(index)}`} className="my-x-small border-stroke" />
        ) : (
          <div
            key={item.title}
            className="relative"
            onMouseEnter={() => {
              setOpen(item.children ? index : null);
            }}
          >
            <button
              type="button"
              role={item.isChecked === undefined ? "menuitem" : "menuitemcheckbox"}
              aria-checked={item.isChecked}
              aria-haspopup={item.children ? "menu" : undefined}
              aria-expanded={item.children ? open === index : undefined}
              disabled={item.isDisabled}
              title={item.hint}
              onKeyDown={(event) => {
                if (item.children && event.key === "ArrowRight") {
                  event.preventDefault();
                  setOpen(index);
                }
              }}
              onClick={() => {
                if (item.children) {
                  setOpen(open === index ? null : index);
                  return;
                }
                onClose();
                item.run();
              }}
              className={cx(
                "flex h-24 w-full cursor-pointer items-center rounded-small px-small text-left text-body whitespace-nowrap outline-none",
                "hover:bg-selection focus:bg-selection disabled:cursor-default disabled:opacity-40",
                item.isDestructive ? "text-danger" : "text-text",
              )}
            >
              {item.isChecked !== undefined && <span aria-hidden className="w-16">{item.isChecked ? "✓" : ""}</span>}
              <span className="flex-1">{item.title}</span>
              {item.children && (
                <span aria-hidden className="pl-medium text-faint">
                  ›
                </span>
              )}
            </button>
            {item.children && open === index && (
              <MenuList items={item.children} onClose={onClose} className="absolute top-0 left-full z-50 ml-x-small" />
            )}
          </div>
        ),
      )}
    </div>
  );
}

export function ContextMenu({
  x,
  y,
  items,
  onClose,
}: {
  x: number;
  y: number;
  items: (MenuItem | "divider")[];
  onClose: () => void;
}) {
  const menu = useRef<HTMLDivElement>(null);
  useEffect(() => {
    menu.current?.querySelector<HTMLElement>("[role=menuitem]")?.focus();
    const close = (event: PointerEvent) => {
      if (!menu.current?.contains(event.target as Node)) onClose();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener("pointerdown", close);
    window.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [onClose]);
  return (
    <div ref={menu}>
      <MenuList items={items} onClose={onClose} className="fixed z-50" style={{ left: x, top: y }} />
    </div>
  );
}

/**
 * Delete from Library…, or Move to Trash… for `images`: Rust says which of `ids` can go (a result only on Higgsfield
 * can't) and asks in its words, then deletes them.
 */
export function DeleteDialog({
  ids,
  images = false,
  onDone,
}: {
  ids: string[];
  images?: boolean;
  onDone: (deleted: boolean) => void;
}) {
  const [question, setQuestion] = useState<{ ids: string[]; title: string; message: string; confirmTitle: string } | null>(
    null,
  );
  const done = useRef(onDone);
  useEffect(() => {
    done.current = onDone;
  });
  const key = ids.join();
  useEffect(() => {
    void commands.libraryBulk(key.split(",")).then(async (bulk) => {
      const deletable = bulk.deletableIds;
      if (deletable.length === 0) {
        done.current(false);
        return;
      }
      const asked = await commands.libraryDeleteConfirmation(deletable.length, images);
      setQuestion({ ids: deletable, ...asked });
    });
  }, [key, images]);
  if (!question) return null;
  return (
    <AlertDialog
      title={question.title}
      message={question.message}
      onCancel={() => {
        onDone(false);
      }}
    >
      <Button
        variant="secondary"
        title="Cancel"
        onClick={() => {
          onDone(false);
        }}
      />
      <Button
        variant="secondary"
        destructive
        title={question.confirmTitle}
        onClick={() => {
          void commands.libraryDelete(question.ids).then(() => {
            onDone(true);
          });
        }}
      />
    </AlertDialog>
  );
}

/** Space: the result or image large, in the window (the in-app preview). Space or Esc closes it. */
export function Preview({
  title,
  url,
  isVideo,
  onClose,
}: {
  title: string;
  url: string | null;
  isVideo: boolean;
  onClose: () => void;
}) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== " ") return;
      event.preventDefault();
      event.stopPropagation();
      onClose();
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
    };
  }, [onClose]);
  return (
    <Dialog title={title} onCancel={onClose} className="max-w-[90vw] items-center">
      <div className="flex w-full items-center gap-small">
        <p className="m-0 mr-auto truncate text-heading font-bold">{title}</p>
        <Button variant="icon" icon={X} title="Close" onClick={onClose} />
      </div>
      {url === null ? (
        <p className="m-0 text-muted">Can't load the preview</p>
      ) : isVideo ? (
        <video src={url} controls autoPlay className="max-h-[75vh] max-w-full rounded-media" />
      ) : (
        <img src={url} alt={title} className="max-h-[75vh] max-w-full rounded-media object-contain" />
      )}
    </Dialog>
  );
}

/** Made from / Used in: each link opens its result page or the image's Details…. */
export function LinkList({
  title,
  links,
  empty,
  onOpenImage,
}: {
  title: string;
  links: LinkView_Serialize[];
  empty?: string;
  onOpenImage: (link: LinkView_Serialize) => void;
}) {
  const navigate = useNavigate();
  if (links.length === 0 && empty === undefined) return null;
  return (
    <section className="flex flex-col gap-small">
      <SectionLabel title={title} detail={links.length > 0 ? String(links.length) : undefined} />
      {links.length === 0 && <p className="m-0 text-control text-muted">{empty}</p>}
      <div className="flex flex-wrap gap-small">
        {links.map((link) => (
          <button
            key={`${link.reference.materialID}-${link.reference.versionID}-${link.roleTitle}`}
            type="button"
            disabled={link.availability === "unavailable"}
            title={link.prompt}
            onClick={() => {
              if (link.isVideo) void navigate({ to: "/library/result/$id", params: { id: link.reference.materialID } });
              else onOpenImage(link);
            }}
            className={cx(
              "flex w-120 cursor-pointer flex-col gap-2 rounded-field border border-stroke bg-well p-x-small text-left",
              "disabled:cursor-default disabled:opacity-50",
              focusRing,
            )}
          >
            <span className="relative aspect-video w-full overflow-hidden rounded-small bg-glass">
              {link.previewUrl !== null &&
                (link.isVideo ? (
                  <video src={link.previewUrl} muted preload="metadata" className="size-full object-cover" />
                ) : (
                  <img src={link.previewUrl} alt="" className="size-full object-cover" />
                ))}
            </span>
            <span className="truncate text-caption font-semibold text-text">{link.roleTitle}</span>
            {link.note !== null && <span className="truncate text-caption text-faint">{link.note}</span>}
          </button>
        ))}
      </div>
    </section>
  );
}

/** An image's Details…: its prompt, what it was made from and the videos made from it. */
export function ImageDetailsDialog({
  id,
  versionId,
  onClose,
}: {
  id: string;
  versionId: string | null;
  onClose: () => void;
}) {
  const [shown, setShown] = useState<{ id: string; versionId: string | null }>({ id, versionId });
  const details = useImageDetails(shown.id, shown.versionId).data;
  return (
    <Dialog title="Details" onCancel={onClose} className="max-w-640">
      {details ? (
        <div className="flex flex-col gap-medium">
          {details.image.url !== null || details.remoteUrl !== null ? (
            <img
              src={details.image.url ?? details.remoteUrl ?? undefined}
              alt=""
              className="max-h-280 w-full rounded-media object-contain"
            />
          ) : (
            <p className="m-0 text-muted">Can't be found</p>
          )}
          <Field title="Prompt">
            <p className={cx("m-0 text-body select-text", details.hasPrompt ? "text-text" : "text-faint")}>
              {details.prompt}
            </p>
          </Field>
          <p className="m-0 text-caption text-muted">
            {[details.model, details.createdText].filter((part) => part !== "").join(" · ")}
          </p>
          <LinkList
            title="Made from"
            links={details.madeFrom}
            onOpenImage={(link) => {
              setShown({ id: link.reference.materialID, versionId: link.reference.versionID });
            }}
          />
          <LinkList
            title="Used in"
            links={details.usedIn}
            empty="No video has been made from this image yet."
            onOpenImage={() => undefined}
          />
        </div>
      ) : null}
      <div className="flex justify-end">
        <Button variant="secondary" title="Done" onClick={onClose} />
      </div>
    </Dialog>
  );
}

export function Field({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-x-small">
      <SectionLabel title={title} />
      {children}
    </section>
  );
}
