import { RotateCcw } from "lucide-react";
import { useEffect, useState } from "react";

import { commands, type ShortcutAction, type ShortcutRow_Serialize } from "../../bindings";
import { cx } from "../../design/cx";
import { Button, focusRing } from "../../design/Styles";
import { useCommandMutation } from "../../lib/ipc";
import { keyStroke } from "../../lib/shortcuts/useShortcut";
import { SettingsRow, SettingsSection } from "./SettingsRows";
import { useErrorAlert, useShortcutSettings } from "./settingsData";

/**
 * Settings › Shortcuts (ShortcutSettingsView.swift): click a shortcut, then press the new keys. Escape cancels,
 * Backspace or Delete removes it. Rust decides what a key press does and stores it.
 */
export function ShortcutsTab() {
  const settings = useShortcutSettings().data;
  const errors = useErrorAlert("Can't change this setting");
  const [recording, setRecording] = useState<ShortcutAction | null>(null);
  const [refusal, setRefusal] = useState<{ action: ShortcutAction; message: string } | null>(null);
  const record = useCommandMutation(
    ({ action, stroke }: { action: ShortcutAction; stroke: ReturnType<typeof keyStroke> }) =>
      commands.recordShortcut(action, stroke),
  );
  const reset = useCommandMutation((action: ShortcutAction) => commands.resetShortcut(action));
  const restore = useCommandMutation(() => commands.restoreDefaultShortcuts());
  const { mutate: send } = record;

  // While recording, every key goes to the recorder first, so recording Ctrl+1 doesn't also switch the page.
  useEffect(() => {
    if (recording === null) return;
    const onKey = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();
      if (event.repeat) return;
      send(
        { action: recording, stroke: keyStroke(event) },
        {
          onSuccess: (result) => {
            if (result.kind === "refused") {
              setRefusal({ action: recording, message: result.message });
            } else if (result.kind !== "ignored") {
              setRefusal(null);
              setRecording(null);
            }
          },
          onError: errors.show,
        },
      );
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => {
      window.removeEventListener("keydown", onKey, { capture: true });
    };
  }, [recording, send, errors.show]);

  if (!settings) return null;

  return (
    <div className="flex flex-col gap-large">
      <SettingsSection
        title="Keyboard"
        footer="Click a shortcut and press the new keys. Escape cancels, Backspace or Delete removes it. Ctrl+, (Settings) and the system's shortcuts stay as they are. Keep, Reject and Clear work only in the Library's grid and result page, so they can be a single key."
      >
        {settings.rows.map((row, index) => (
          <SettingsRow
            key={row.action}
            label={row.title}
            detail={refusal?.action === row.action ? refusal.message : undefined}
            isFirst={index === 0}
          >
            <div className="flex items-center gap-small">
              {row.isChanged && (
                <Button
                  variant="icon"
                  icon={RotateCcw}
                  title={`Reset ${row.title}`}
                  hint={row.resetHint}
                  onClick={() => {
                    setRefusal(null);
                    reset.mutate(row.action, { onError: errors.show });
                  }}
                />
              )}
              <Recorder
                row={row}
                isRecording={recording === row.action}
                onClick={() => {
                  setRefusal(null);
                  setRecording(recording === row.action ? null : row.action);
                }}
              />
            </div>
          </SettingsRow>
        ))}
      </SettingsSection>
      <div>
        <Button
          variant="secondary"
          title="Restore Defaults"
          disabled={!settings.canRestoreDefaults}
          onClick={() => {
            setRefusal(null);
            restore.mutate(undefined, { onError: errors.show });
          }}
        />
      </div>
      {errors.alert}
    </div>
  );
}

function Recorder({
  row,
  isRecording,
  onClick,
}: {
  row: ShortcutRow_Serialize;
  isRecording: boolean;
  onClick: () => void;
}) {
  const current = row.shortcut ?? "None";
  return (
    <button
      type="button"
      aria-label={`${row.title} shortcut`}
      aria-description={current}
      aria-pressed={isRecording}
      onClick={onClick}
      className={cx(
        "min-h-28 min-w-120 cursor-pointer rounded-field border bg-well px-10 text-body font-medium",
        focusRing,
        isRecording
          ? "border-accent text-accent shadow-[inset_0_0_0_0.5px_var(--color-accent)]"
          : "border-stroke",
        !isRecording && (row.shortcut === null ? "text-faint" : "text-text"),
      )}
    >
      {isRecording ? "Type shortcut…" : current}
    </button>
  );
}
