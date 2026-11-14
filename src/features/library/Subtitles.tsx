import { Captions, MessageSquareText, Minimize2, Scissors } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { AlertDialog, Dialog } from "../../app/Dialog";
import {
  commands,
  type SubtitleEdit,
  type SubtitleEditorView_Serialize,
  type SubtitleEngine,
} from "../../bindings";
import { SectionLabel } from "../../design/Components";
import { cx } from "../../design/cx";
import { SegmentedControl } from "../../design/Pickers";
import { Button, focusRing } from "../../design/Styles";
import { isEditingText } from "../../design/textFocus";
import { useShownSheet } from "../../app/agentShow";
import { useCommand } from "../../lib/ipc";
import { queryKeys } from "../../lib/queryKeys";
import { errorText } from "../create/createData";

/**
 * A result page's subtitles (`SubtitlesSection`): the run making them, with Cancel, or what the current version has,
 * Edit Subtitles and Subtitles… (Make Subtitles Again… once it has some). Every text comes from Rust.
 */
export function SubtitlesSection({ id, title, onEdit }: { id: string; title: string; onEdit: () => void }) {
  const sheet = useCommand(queryKeys.subtitles.sheet(id, null, null), () => commands.subtitlesSheet(id, null, null)).data;
  const [isMaking, setIsMaking] = useState(false);
  if (!sheet) return null;
  const run = sheet.run;

  return (
    <div className="flex flex-col gap-small">
      {run !== null ? (
        <div className="flex items-center gap-small" role="status" aria-label={run.progress}>
          <ProgressBar fraction={run.fraction} />
          <span className="shrink-0 truncate text-small text-muted">{run.progress}</span>
          <Button variant="textLink" title="Cancel" onClick={() => void commands.subtitlesCancel(run.id)} />
        </div>
      ) : (
        <>
          {sheet.current !== null && (
            <>
              <span className="text-small text-muted">{sheet.current}</span>
              <Button variant="secondary" icon={MessageSquareText} title="Edit Subtitles" className="w-full" onClick={onEdit} />
            </>
          )}
          <Button
            variant="secondary"
            icon={Captions}
            title={sheet.current === null ? "Subtitles…" : "Make Subtitles Again…"}
            className="w-full"
            onClick={() => {
              setIsMaking(true);
            }}
          />
        </>
      )}
      {sheet.failure !== null && (
        <AlertDialog
          title={sheet.failure.title}
          message={sheet.failure.message}
          onCancel={() => void commands.subtitlesTakeFailure()}
        >
          <Button variant="primary" title="OK" onClick={() => void commands.subtitlesTakeFailure()} />
        </AlertDialog>
      )}
      {isMaking && (
        <SubtitlesSheet
          id={id}
          title={title}
          onClose={() => {
            setIsMaking(false);
          }}
        />
      )}
    </div>
  );
}

/** A run's bar, or a spinner-like empty track while it waits. */
function ProgressBar({ fraction }: { fraction: number | null }) {
  return (
    <span className="h-4 min-w-60 flex-1 overflow-hidden rounded-full bg-track">
      <span
        className={cx("block h-full rounded-full bg-accent", fraction === null && "animate-pulse")}
        style={{ width: `${String((fraction ?? 1) * 100)}%` }}
      />
    </span>
  );
}

/**
 * The Subtitles sheet (`SubtitlesSheet.swift`): the language and the model, the agreement to the model's download
 * before its first run, that it is free and what it is made from, then Make Subtitles.
 */
export function SubtitlesSheet({ id, title, onClose }: { id: string; title: string; onClose: () => void }) {
  const [language, setLanguage] = useState<string | null>(null);
  const [engine, setEngine] = useState<SubtitleEngine | null>(null);
  const [agrees, setAgrees] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sheet = useCommand(queryKeys.subtitles.sheet(id, language, engine), () =>
    commands.subtitlesSheet(id, language, engine),
  ).data;
  if (!sheet) return null;
  const picked = language ?? sheet.language;
  const canMake =
    picked !== null && sheet.run === null && sheet.unavailable === null && (sheet.modelNotice === null || agrees);

  const make = () => {
    if (picked === null) return;
    commands
      .subtitlesStart(id, picked, sheet.engine, agrees)
      .then(onClose)
      .catch((failure: unknown) => {
        setError(errorText(failure));
      });
  };

  return (
    <Dialog title={sheet.title} onCancel={onClose} className="w-440 gap-large p-24" sheet="subtitles">
      <div className="flex flex-col gap-4">
        <h2 className="m-0 text-brand font-bold text-text">{sheet.title}</h2>
        <p className="m-0 truncate text-control text-muted">{title}</p>
      </div>
      <SectionLabel title="Language" />
      <select
        aria-label="Language"
        className={cx("h-28 self-start rounded-small border border-stroke bg-glass px-small text-body text-text", focusRing)}
        value={picked ?? ""}
        onChange={(event) => {
          // The person agrees per language: a model downloads per language.
          setAgrees(false);
          setLanguage(event.target.value);
        }}
        disabled={sheet.languages.length === 0}
      >
        {sheet.languages.map((option) => (
          <option key={option.value} value={option.value}>
            {option.title}
          </option>
        ))}
      </select>
      <SectionLabel title="Model" />
      <SegmentedControl
        isCompact
        className="self-start"
        value={sheet.engine}
        options={sheet.engines.map((option) => ({ value: option.value as SubtitleEngine, title: option.title }))}
        onChange={setEngine}
      />
      {sheet.unavailable !== null && <p className="m-0 text-small text-warning">{sheet.unavailable}</p>}
      {sheet.modelNotice !== null && (
        <div className="flex flex-col gap-small">
          <p className="m-0 text-small text-muted">{sheet.modelNotice}</p>
          <label className="flex items-center gap-6 text-body">
            <input
              type="checkbox"
              checked={agrees}
              onChange={(event) => {
                setAgrees(event.target.checked);
              }}
            />
            {sheet.modelConsent}
          </label>
        </div>
      )}
      <div className="flex flex-col gap-small text-small text-muted">
        <p className="m-0">{sheet.price}</p>
        <p className="m-0">{sheet.soundNote}</p>
        <p className="m-0">{sheet.versionNote}</p>
      </div>
      {error !== null && (
        <p className="m-0 text-body text-danger" role="alert">
          {error}
        </p>
      )}
      <div className="flex justify-end gap-small">
        <Button variant="secondary" title="Cancel" onClick={onClose} />
        <Button variant="primary" icon={Captions} title="Make Subtitles" price="Free" disabled={!canMake} onClick={make} />
      </div>
    </Dialog>
  );
}

/**
 * The subtitle editor (`SubtitleEditorPage`), in place of the result page: the video with the cue on screen over it,
 * and beside it the cues with their times and text, Split and Merge with Next. Done saves only over what it opened
 * with, unless the person replaces subtitles made meanwhile; Cancel leaves them as they were.
 */
export function SubtitleEditor({
  id,
  videoUrl,
  aspect,
  onClose,
}: {
  id: string;
  videoUrl: string | null;
  aspect: number;
  onClose: () => void;
}) {
  const loaded = useCommand(queryKeys.subtitles.editor(id), () => commands.subtitlesEditor(id)).data;
  // The editor keeps what it opened with while a run elsewhere fetches the view again: Done compares with it.
  const [opened, setOpened] = useState<SubtitleEditorView_Serialize | null>(null);
  if (opened === null && loaded) setOpened(loaded);
  if (!opened) return null;
  return <EditorBody opened={opened} videoUrl={videoUrl} aspect={aspect} onClose={onClose} />;
}

function EditorBody({
  opened,
  videoUrl,
  aspect,
  onClose,
}: {
  opened: SubtitleEditorView_Serialize;
  videoUrl: string | null;
  aspect: number;
  onClose: () => void;
}) {
  const [view, setView] = useState(opened);
  const [time, setTime] = useState(0);
  const [isPlaying, setIsPlaying] = useState(false);
  const [conflict, setConflict] = useState<{ title: string; message: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const player = useRef<HTMLVideoElement>(null);
  const list = useRef<HTMLOListElement>(null);
  const subtitles = view.subtitles;
  const currentIndex = subtitles.cues.findIndex(
    (cue) => cue.start !== null && cue.end !== null && time >= cue.start && time < cue.end,
  );
  const current = currentIndex >= 0 ? subtitles.cues[currentIndex] : undefined;

  const edit = (change: SubtitleEdit) => {
    void commands.subtitlesEdit(view.materialId, view.versionId, subtitles, change).then(setView);
  };
  const save = (replace: boolean) => {
    void commands
      .subtitlesSave(view.materialId, view.versionId, opened.subtitles, subtitles, replace)
      .then((outcome) => {
        if (outcome.kind === "saved") onClose();
        else if (outcome.kind === "changedMeanwhile") setConflict(outcome);
        else setError(outcome.message.message);
      });
  };

  // While it plays, the list follows the cue on screen.
  useEffect(() => {
    if (!isPlaying || currentIndex < 0) return;
    list.current?.children.item(currentIndex)?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [currentIndex, isPlaying]);

  // Esc cancels and Ctrl+Enter is Done, as the sheet's keys; a text field keeps its own keys.
  useEffect(() => {
    if (conflict !== null || error !== null) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !isEditingText()) {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  });

  return (
    <section aria-label={view.title} className="flex h-full min-h-0 flex-col gap-large px-page pt-page-top pb-page">
      <div className="flex items-center gap-small">
        <div className="mr-auto flex flex-col gap-2">
          <h1 className="m-0 text-brand font-bold text-text">{view.title}</h1>
          <span className="text-small text-muted">{view.summary}</span>
        </div>
        <Button variant="secondary" title="Cancel" onClick={onClose} />
        <Button
          variant="primary"
          title="Done"
          onClick={() => {
            save(false);
          }}
        />
      </div>
      <div className="flex min-h-0 flex-1 items-start gap-x-large">
        <div className="flex min-h-0 min-w-0 flex-1 items-center justify-center self-stretch">
          {videoUrl !== null && (
            <div className="relative max-h-full w-full overflow-hidden rounded-small bg-black" style={{ aspectRatio: aspect }}>
              <video
                ref={player}
                src={videoUrl}
                controls
                className="absolute inset-0 size-full object-contain"
                onTimeUpdate={(event) => {
                  setTime(event.currentTarget.currentTime);
                }}
                onPlay={() => {
                  setIsPlaying(true);
                }}
                onPause={() => {
                  setIsPlaying(false);
                }}
              />
              {current !== undefined && (
                // The cue on screen, as a burned-in subtitle looks: white, semibold, with a soft shadow.
                <p
                  aria-hidden
                  className="pointer-events-none absolute inset-x-[5%] bottom-[8%] m-0 text-center font-semibold whitespace-pre-line text-white [text-shadow:0_0_6px_rgb(0_0_0/0.85)]"
                  style={{ fontSize: "max(4.5cqh, 11px)" }}
                >
                  {current.text}
                </p>
              )}
            </div>
          )}
        </div>
        <ol ref={list} className="m-0 flex max-h-full w-340 shrink-0 list-none flex-col overflow-y-auto p-0" aria-label="Cues">
          {view.cues.map((cue, index) => {
            const timed = subtitles.cues[index];
            const start = timed?.start ?? null;
            return (
              <li
                key={cue.id}
                className={cx("flex flex-col gap-4 rounded-small p-8", index === currentIndex && "bg-well")}
              >
                <div className="flex items-center gap-small">
                  <button
                    type="button"
                    className={cx("mr-auto cursor-pointer text-caption text-accent tabular-nums hovered:underline", focusRing)}
                    title="Play from here"
                    onClick={() => {
                      if (player.current && start !== null) {
                        player.current.currentTime = start;
                        setTime(start);
                      }
                    }}
                  >
                    {cue.time}
                  </button>
                  <IconButton
                    title="Split"
                    hint="Split this cue in two"
                    icon={<Scissors aria-hidden size={13} />}
                    disabled={!cue.canSplit}
                    onClick={() => {
                      edit({ kind: "split", id: cue.id });
                    }}
                  />
                  <IconButton
                    title="Merge with Next"
                    hint="Merge with the next cue"
                    icon={<Minimize2 aria-hidden size={13} />}
                    disabled={!cue.canMerge}
                    onClick={() => {
                      edit({ kind: "mergeWithNext", id: cue.id });
                    }}
                  />
                </div>
                <textarea
                  aria-label={cue.time}
                  placeholder="Text"
                  className="w-full resize-none border-0 bg-transparent p-0 text-body text-text outline-none"
                  rows={Math.min(Math.max((timed?.text ?? "").split("\n").length, 1), 4)}
                  defaultValue={timed?.text ?? ""}
                  key={`${cue.id}-${timed?.text ?? ""}`}
                  onBlur={(event) => {
                    if (event.target.value !== timed?.text) edit({ kind: "setText", id: cue.id, text: event.target.value });
                  }}
                />
              </li>
            );
          })}
        </ol>
      </div>
      {error !== null && (
        <AlertDialog
          title="Couldn't save the subtitles"
          message={error}
          onCancel={() => {
            setError(null);
          }}
        >
          <Button
            variant="primary"
            title="OK"
            onClick={() => {
              setError(null);
            }}
          />
        </AlertDialog>
      )}
      {conflict !== null && (
        <AlertDialog
          title={conflict.title}
          message={conflict.message}
          onCancel={() => {
            setConflict(null);
          }}
        >
          <Button variant="secondary" title="Keep the New Ones" onClick={onClose} />
          <Button
            variant="secondary"
            destructive
            title="Replace with Mine"
            onClick={() => {
              setConflict(null);
              save(true);
            }}
          />
        </AlertDialog>
      )}
    </section>
  );
}

/** A borderless icon button of a cue row. */
function IconButton({
  title,
  hint,
  icon,
  disabled,
  onClick,
}: {
  title: string;
  hint: string;
  icon: React.ReactNode;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={title}
      title={hint}
      disabled={disabled}
      onClick={onClick}
      className={cx(
        "flex size-20 cursor-pointer items-center justify-center rounded-small text-muted hovered:text-text disabled:cursor-default disabled:opacity-40",
        focusRing,
      )}
    >
      {icon}
    </button>
  );
}

/** The Subtitles sheet, when an agent's `show` asked for it on the item `id`. */
export function AgentShownSubtitlesSheet({ id, title }: { id: string; title: string }) {
  const { item, sheet, set } = useShownSheet();
  if (item !== id || sheet !== "subtitles") return null;
  return (
    <SubtitlesSheet
      id={id}
      title={title}
      onClose={() => {
        set(null, null);
      }}
    />
  );
}
