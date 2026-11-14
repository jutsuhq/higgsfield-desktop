import { useState } from "react";

import { AlertDialog } from "../../app/Dialog";
import { commands, type AppError, type Preferences_Deserialize } from "../../bindings";
import { Button } from "../../design/Styles";
import { useCommand, useCommandMutation } from "../../lib/ipc";
import { queryKeys } from "../../lib/queryKeys";

/** What the tabs show, computed in Rust; the preference-backed ones follow `PreferencesChanged`. */
export const useGeneralSettings = () =>
  useCommand(queryKeys.settings.general, () => commands.generalSettings());
export const useOutputSettings = () => useCommand(queryKeys.settings.output, () => commands.outputSettings());
export const useAgentSettings = () => useCommand(queryKeys.settings.agents, () => commands.agentsSettings());
export const useNotificationSettings = () =>
  useCommand(queryKeys.settings.notifications, () => commands.notificationSettings());
export const useShortcutSettings = () =>
  useCommand(queryKeys.settings.shortcuts, () => commands.shortcutSettings());
export const useSubtitleSettings = () =>
  useCommand(queryKeys.settings.subtitles, () => commands.subtitleSettings());

/** Writes a part of the settings; `PreferencesChanged` brings the tab up to date. */
export function useSetPreferences(onError?: (error: AppError) => void) {
  const mutation = useCommandMutation((patch: Preferences_Deserialize) => commands.setPreferences(patch));
  return (patch: Preferences_Deserialize) => {
    mutation.mutate(patch, { onError });
  };
}

/** A failed change: "Can't change this setting" with what went wrong, until OK. */
export function useErrorAlert(title: string) {
  const [message, setMessage] = useState<string | null>(null);
  const show = (error: AppError) => {
    setMessage(error.message);
  };
  const alert =
    message === null ? null : (
      <ErrorAlert
        title={title}
        message={message}
        onClose={() => {
          setMessage(null);
        }}
      />
    );
  return { show, alert };
}

export function ErrorAlert({
  title,
  message,
  onClose,
}: {
  title: string;
  message: string;
  onClose: () => void;
}) {
  return (
    <AlertDialog title={title} message={message} onCancel={onClose}>
      <Button variant="secondary" title="OK" onClick={onClose} />
    </AlertDialog>
  );
}

/** A confirmation with Cancel and a destructive action. */
export function ConfirmDialog({
  title,
  message,
  action,
  onConfirm,
  onCancel,
}: {
  title: string;
  message: string;
  action: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <AlertDialog title={title} message={message} onCancel={onCancel}>
      <Button variant="secondary" title="Cancel" onClick={onCancel} />
      <Button
        variant="secondary"
        destructive
        title={action}
        onClick={() => {
          onCancel();
          onConfirm();
        }}
      />
    </AlertDialog>
  );
}
