import { Link } from "lucide-react";
import { useState } from "react";
import { create } from "zustand";

import { commands, type LinkDownloadView_Serialize, type LinkEntryView } from "../../bindings";
import { Dialog } from "../../app/Dialog";
import { Button, TextField } from "../../design/Styles";
import { showError, useCreateNotice } from "./createData";

type LinkKind = "video" | "image";

interface LinkEntryState {
  /** The Paste URL field being shown, and the section that asked. */
  entry: { kind: LinkKind; view: LinkEntryView } | null;
  close: () => void;
}

const useLinkEntry = create<LinkEntryState>((set) => ({
  entry: null,
  close: () => {
    set({ entry: null });
  },
}));

/** What the clipboard holds as text; nothing when the webview may not read it. */
async function clipboardText(): Promise<string> {
  try {
    return await navigator.clipboard.readText();
  } catch {
    return "";
  }
}

/** Opens the Paste URL field for `kind`, filled with the clipboard's link if it holds one. */
export function askForLink(kind: LinkKind) {
  void clipboardText()
    .then((text) => commands.createLinkEntry(kind, text))
    .then(
      (view) => {
        useLinkEntry.setState({ entry: { kind, view } });
      },
      (error: unknown) => {
        showError("Can't use this link", error);
      },
    );
}

/** Downloads `text` as `kind` (`null` takes either), then takes the file like a dropped one. */
export function downloadLink(text: string, kind: LinkKind | null) {
  commands.createDownloadLink(text, kind).then(
    (outcome) => {
      const notice = useCreateNotice.getState();
      if (outcome.kind === "failed")
        notice.show({ title: outcome.message.title, detail: outcome.message.message });
      else if (outcome.kind === "done" && outcome.notice !== null)
        notice.show({ title: outcome.notice.title, detail: outcome.notice.message });
    },
    (error: unknown) => {
      showError("Can't download the file", error);
    },
  );
}

/** "Paste URL…" next to a section's title. */
export function PasteLinkButton({ kind, isDisabled }: { kind: LinkKind; isDisabled: boolean }) {
  return (
    <Button
      variant="icon"
      icon={Link}
      title="Paste URL…"
      hint="Download from a link"
      className="size-26"
      disabled={isDisabled}
      onClick={() => {
        askForLink(kind);
      }}
    />
  );
}

/** A link being downloaded: its host, progress and Cancel. */
export function LinkDownloadRow({ download }: { download: LinkDownloadView_Serialize }) {
  return (
    <div className="flex items-center gap-10 rounded-field border border-stroke bg-well p-medium">
      <div className="flex min-w-0 flex-1 flex-col gap-6">
        <span className="truncate text-caption text-muted tabular-nums">{download.text}</span>
        <progress
          aria-label={download.text}
          className="h-4 w-full accent-accent"
          {...(download.fraction === null ? {} : { value: download.fraction, max: 1 })}
        />
      </div>
      <Button
        variant="secondary"
        title="Cancel"
        onClick={() => {
          void commands.createCancelLink();
        }}
      />
    </div>
  );
}

/** The Paste URL field: a link, Download and Cancel. */
export function LinkEntryDialog() {
  const { entry, close } = useLinkEntry();
  if (entry === null) return null;
  return <LinkEntryForm key={entry.view.text} kind={entry.kind} view={entry.view} onClose={close} />;
}

function LinkEntryForm({
  kind,
  view,
  onClose,
}: {
  kind: LinkKind;
  view: LinkEntryView;
  onClose: () => void;
}) {
  const [text, setText] = useState(view.text);
  return (
    <Dialog title={view.title} role="alertdialog" onCancel={onClose}>
      <form
        className="flex flex-col gap-16"
        onSubmit={(event) => {
          event.preventDefault();
          onClose();
          downloadLink(text, kind);
        }}
      >
        <div className="flex flex-col gap-6">
          <p className="m-0 text-heading font-bold">{view.title}</p>
          <p className="m-0 text-body text-muted">{view.explanation}</p>
        </div>
        <TextField
          aria-label={view.title}
          placeholder={view.placeholder}
          data-autofocus
          value={text}
          onChange={(event) => {
            setText(event.target.value);
          }}
        />
        <div className="flex justify-end gap-small">
          <Button variant="secondary" title="Cancel" onClick={onClose} />
          <Button variant="primary" type="submit" title="Download" />
        </div>
      </form>
    </Dialog>
  );
}
