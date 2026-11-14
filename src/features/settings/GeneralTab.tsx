import { useState } from "react";

import { AlertDialog } from "../../app/Dialog";
import { commands, type Mode, type UpdateCheck, type UpdateSwitch } from "../../bindings";
import { SegmentedControl } from "../../design/Pickers";
import { Button } from "../../design/Styles";
import { useCommandMutation } from "../../lib/ipc";
import { queryKeys } from "../../lib/queryKeys";
import { SettingsRow, SettingsSection, Switch } from "./SettingsRows";
import {
  ConfirmDialog,
  ErrorAlert,
  useErrorAlert,
  useGeneralSettings,
  useSetPreferences,
} from "./settingsData";

/**
 * Settings › General (GeneralSettingsView.swift): the mode Create starts in, prompt history, the system tray, opening
 * at login and Software Update. A build without an updater dims its switches, and Check Now says so; an update
 * Check Now finds is installed once the person agrees.
 */
export function GeneralTab() {
  const settings = useGeneralSettings().data;
  const errors = useErrorAlert("Can't change this setting");
  const setPreferences = useSetPreferences(errors.show);
  const [isConfirmingClear, setConfirmingClear] = useState(false);
  const [updateCheck, setUpdateCheck] = useState<UpdateCheck | null>(null);
  const clearHistory = useCommandMutation(() => commands.clearPromptHistory());
  const setOpenAtLogin = useCommandMutation(
    (isOn: boolean) => commands.setOpenAtLogin(isOn),
    [queryKeys.settings.general],
  );
  const checkForUpdates = useCommandMutation(() => commands.checkForUpdates(), [queryKeys.settings.general]);
  const installUpdate = useCommandMutation(() => commands.installUpdate());
  const setUpdateSwitch = useCommandMutation(({ which, isOn }: { which: UpdateSwitch; isOn: boolean }) =>
    commands.setUpdateSwitch(which, isOn),
  );
  if (!settings) return null;

  return (
    <div className="flex flex-col gap-x-large">
      <SettingsSection title="Create" footer="The mode Create starts in when Jutsu Studio opens.">
        <SettingsRow label="Default mode" isFirst>
          <SegmentedControl<Mode>
            isCompact
            value={settings.defaultMode}
            options={settings.modes}
            onChange={(mode) => {
              setPreferences({ "settings.defaultMode": mode });
            }}
          />
        </SettingsRow>
        <SettingsRow label="Prompt history" detail="Starred prompts stay">
          <Button
            variant="secondary"
            title="Clear History…"
            onClick={() => {
              setConfirmingClear(true);
            }}
          />
        </SettingsRow>
      </SettingsSection>

      <SettingsSection title="System" footer={settings.systemFooter}>
        <SettingsRow label="Show in system tray" isFirst isDisabled={!settings.canChangeShowInTray}>
          <Switch
            label="Show in system tray"
            isOn={settings.showInTray}
            disabled={!settings.canChangeShowInTray}
            onChange={(isOn) => {
              setPreferences({ "settings.showInMenuBar": isOn });
            }}
          />
        </SettingsRow>
        <SettingsRow label="Run in the system tray" detail="Stays open with no window">
          <Switch
            label="Run in the system tray"
            isOn={settings.runInTray}
            onChange={(isOn) => {
              setPreferences({ "settings.runInMenuBar": isOn });
            }}
          />
        </SettingsRow>
        <SettingsRow
          label="Active job count"
          detail="Next to the system tray icon"
          isDisabled={!settings.canShowActiveCount}
        >
          <Switch
            label="Active job count"
            isOn={settings.showActiveCount}
            disabled={!settings.canShowActiveCount}
            onChange={(isOn) => {
              setPreferences({ "settings.showActiveCount": isOn });
            }}
          />
        </SettingsRow>
        <SettingsRow label="Open at login">
          <Switch
            label="Open at login"
            isOn={settings.openAtLogin}
            onChange={(isOn) => {
              setOpenAtLogin.mutate(isOn, { onError: errors.show });
            }}
          />
        </SettingsRow>
      </SettingsSection>

      <SettingsSection title="Updates" footer={settings.updatesFooter}>
        <SettingsRow label="Check automatically" detail="Once a day" isFirst>
          <Switch
            label="Check automatically"
            isOn={settings.checksAutomatically}
            disabled={!settings.updatesEnabled}
            onChange={(isOn) => {
              setUpdateSwitch.mutate({ which: "checksAutomatically", isOn }, { onError: errors.show });
            }}
          />
        </SettingsRow>
        <SettingsRow
          label="Download and install automatically"
          detail="Installs when you quit"
          isDisabled={!settings.checksAutomatically}
        >
          <Switch
            label="Download and install automatically"
            isOn={settings.downloadsAutomatically}
            disabled={!settings.canChangeDownloadsAutomatically}
            onChange={(isOn) => {
              setUpdateSwitch.mutate({ which: "downloadsAutomatically", isOn }, { onError: errors.show });
            }}
          />
        </SettingsRow>
        <SettingsRow label={settings.versionText} detail={settings.lastCheckText}>
          <Button
            variant="secondary"
            title="Check Now"
            disabled={!settings.canCheckForUpdates || checkForUpdates.isPending}
            onClick={() => {
              checkForUpdates.mutate(undefined, { onSuccess: setUpdateCheck, onError: errors.show });
            }}
          />
        </SettingsRow>
      </SettingsSection>

      {isConfirmingClear && (
        <ConfirmDialog
          title="Clear prompt history?"
          message="Prompts of earlier jobs leave the prompt picker. Starred prompts, jobs and the library stay."
          action="Clear History"
          onCancel={() => {
            setConfirmingClear(false);
          }}
          onConfirm={() => {
            clearHistory.mutate(undefined, { onError: errors.show });
          }}
        />
      )}
      {updateCheck?.installTitle != null && (
        <AlertDialog
          title={updateCheck.title}
          message={updateCheck.message}
          onCancel={() => {
            setUpdateCheck(null);
          }}
        >
          <Button
            variant="secondary"
            title="Later"
            onClick={() => {
              setUpdateCheck(null);
            }}
          />
          <Button
            variant="primary"
            title={updateCheck.installTitle}
            onClick={() => {
              setUpdateCheck(null);
              installUpdate.mutate(undefined, { onError: errors.show });
            }}
          />
        </AlertDialog>
      )}
      {updateCheck && updateCheck.installTitle == null && (
        <ErrorAlert
          title={updateCheck.title}
          message={updateCheck.message}
          onClose={() => {
            setUpdateCheck(null);
          }}
        />
      )}
      {errors.alert}
    </div>
  );
}
