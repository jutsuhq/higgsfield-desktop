import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect, useState } from "react";

import { commands, type CreateSide } from "../../bindings";
import { AlertDialog } from "../../app/Dialog";
import { PageHeader } from "../../design/Components";
import { cx } from "../../design/cx";
import { SegmentedControl } from "../../design/Pickers";
import { Button, pagePadding } from "../../design/Styles";
import { chooseSource, showError, useCreateEdit, useCreateNotice, useCreateSnapshot } from "./createData";
import { addCharacterSet, characterDropTarget } from "./CharacterSets";
import { CreatePanel, speechDropTarget } from "./CreatePanel";
import { useImagesSnapshot, useSetCreateSide } from "./imageData";
import { downloadLink, LinkEntryDialog } from "./Links";
import { MotionLibraryBrowser, useMotionLibrary } from "./MotionLibraryBrowser";
import { ImageSide, imageInputDropTarget } from "./ImageSide";
import { EmptyStage, SourceStage } from "./Stage";
import { TemplatesMenu } from "./TemplatesMenu";

const kinds: { value: CreateSide; title: string }[] = [
  { value: "video", title: "Video" },
  { value: "image", title: "Image" },
];

const motionTagline = "Move your characters, products or clothes with motion from any clip.";

function addedNotice(notice: string | null) {
  if (notice !== null) useCreateNotice.getState().show({ title: "Some files weren't added", detail: notice });
}

/** The `data-drop` target under a point of the window, in physical pixels as the webview reports drops. */
function dropTarget(position: { x: number; y: number }): string | null {
  const scale = window.devicePixelRatio || 1;
  const element = document.elementFromPoint(position.x / scale, position.y / scale);
  return element?.closest<HTMLElement>("[data-drop]")?.dataset.drop ?? null;
}

/**
 * Files dropped on the window: on the video side a video becomes the source and images become references; images
 * dropped on the batch characters make one new character. On the image side only the Image to edit row takes a drop.
 */
function useFileDrop(isVideo: boolean): boolean {
  const [isOver, setIsOver] = useState(false);
  useEffect(() => {
    let isDisposed = false;
    let stop: (() => void) | undefined;
    try {
      getCurrentWebview()
        .onDragDropEvent(({ payload }) => {
          if (payload.type === "enter" || payload.type === "over") setIsOver(isVideo);
          else if (payload.type === "leave") setIsOver(false);
          else {
            setIsOver(false);
            const target = dropTarget(payload.position);
            if (target === imageInputDropTarget) {
              commands.imagesEdit({ kind: "dropInput", paths: payload.paths }).catch((error: unknown) => {
                showError("Can't add the image", error);
              });
              return;
            }
            if (!isVideo) return;
            if (target === characterDropTarget) {
              addCharacterSet(payload.paths);
              return;
            }
            if (target === speechDropTarget) {
              commands.createAddSpeech(payload.paths).then(
                (notice) => {
                  if (notice !== null) useCreateNotice.getState().show(notice);
                },
                (error: unknown) => {
                  showError("Can't add the recording", error);
                },
              );
              return;
            }
            commands.createDropFiles(payload.paths).then(
              (outcome) => {
                addedNotice(outcome.notice);
              },
              (error: unknown) => {
                showError("Can't add the files", error);
              },
            );
          }
        })
        .then(
          (unlisten) => {
            if (isDisposed) unlisten();
            else stop = unlisten;
          },
          () => undefined,
        );
    } catch {
      // Outside the app (tests, the design page) there is no webview to listen to.
    }
    return () => {
      isDisposed = true;
      stop?.();
    };
  }, [isVideo]);
  return isOver;
}

/** A link pasted outside a text field is downloaded and taken like a dropped file. */
function pasteLink(event: ClipboardEvent) {
  const target = event.target;
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) return;
  const text = event.clipboardData?.getData("text/plain").trim() ?? "";
  if (!text.startsWith("https://") && !text.startsWith("http://")) return;
  event.preventDefault();
  downloadLink(text, null);
}

/** An image pasted anywhere on Create is saved under `pasted/` and added as a reference. */
function usePasteImage(isEnabled: boolean): void {
  useEffect(() => {
    if (!isEnabled) return;
    const onPaste = (event: ClipboardEvent) => {
      const file = Array.from(event.clipboardData?.files ?? []).find((item) =>
        item.type.startsWith("image/"),
      );
      if (!file) {
        pasteLink(event);
        return;
      }
      event.preventDefault();
      file
        .arrayBuffer()
        .then((buffer) => commands.createPasteImage(Array.from(new Uint8Array(buffer))))
        .then(
          (outcome) => {
            addedNotice(outcome.notice);
          },
          (error: unknown) => {
            showError("Can't paste the image", error);
          },
        );
    };
    window.addEventListener("paste", onPaste);
    return () => {
      window.removeEventListener("paste", onPaste);
    };
  }, [isEnabled]);
}

/**
 * Create (CreateView.swift): the Video | Image switch with the Templates menu, then the side in front. Rust keeps the
 * side in the image draft.
 */
export function CreatePage() {
  const snapshot = useCreateSnapshot().data ?? null;
  const images = useImagesSnapshot().data ?? null;
  const edit = useCreateEdit();
  const setCreateSide = useSetCreateSide();
  const side: CreateSide = images?.side ?? "video";
  const isVideo = side === "video";
  const isOver = useFileDrop(isVideo);
  usePasteImage(isVideo);
  // File › Open… (Ctrl+O) is the window's (`AppShell`): videos and images, any number, on every side.
  const showsLibrary = isVideo && !snapshot?.source && snapshot?.showsMotionLibrary === true;
  const library = useMotionLibrary(showsLibrary).data ?? null;

  return (
    // Either side fills the window: the video panel's form scrolls inside it (`CreateView.videoSide`), and so does
    // the image panel when the window is too short for it (`CreateImagePanel`).
    <div className={cx("flex min-h-0 flex-1 flex-col", pagePadding)}>
      <PageHeader
        title="Create"
        subtitle={isVideo ? (snapshot?.tagline ?? motionTagline) : images?.tagline}
        trailing={
          <div className="flex items-center gap-small">
            <SegmentedControl isCompact value={side} options={kinds} onChange={setCreateSide} />
            <TemplatesMenu side={side} />
          </div>
        }
      />
      {!isVideo ? (
        images && <ImageSide snapshot={images} />
      ) : (
        <div className="flex min-h-0 flex-1 gap-large">
          {/* The panel on the left, 320–380 wide (`CreateView.videoSide`), then the stage. */}
          {snapshot && (
            <aside
              aria-label="Settings"
              className="flex w-380 min-w-320 flex-col overflow-hidden rounded-card border border-stroke bg-card"
            >
              <CreatePanel snapshot={snapshot} edit={edit} />
            </aside>
          )}
          {snapshot?.source ? (
            <SourceStage source={snapshot.source} trim={snapshot.trim} edit={edit} />
          ) : showsLibrary && library ? (
            <MotionLibraryBrowser library={library} onPickFile={chooseSource} />
          ) : (
            <EmptyStage onPick={chooseSource} isLit={isOver} />
          )}
        </div>
      )}
      <Notices />
      <LinkEntryDialog />
    </div>
  );
}

/** A notice to read, or the price question Generate asks before queueing. */
function Notices() {
  const { notice, confirmation, dismiss } = useCreateNotice();
  if (confirmation) {
    return (
      <AlertDialog
        title={confirmation.question.title}
        message={confirmation.question.message}
        onCancel={dismiss}
      >
        <Button variant="secondary" title="Cancel" onClick={dismiss} />
        <Button
          variant="primary"
          title={confirmation.question.confirmTitle}
          onClick={() => {
            dismiss();
            confirmation.approve();
          }}
        />
      </AlertDialog>
    );
  }
  if (notice) {
    return (
      <AlertDialog title={notice.title} message={notice.detail} onCancel={dismiss}>
        <Button variant="primary" title="OK" onClick={dismiss} />
      </AlertDialog>
    );
  }
  return null;
}
