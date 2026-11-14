import { useQuery } from "@tanstack/react-query";

import { commands, type AppError, type BlockedBySystem_Serialize } from "../../bindings";
import { Button } from "../../design/Styles";
import { queryKeys } from "../../lib/queryKeys";
import { SettingsRow, SettingsSection, Switch } from "./SettingsRows";
import { useErrorAlert, useNotificationSettings, useSetPreferences } from "./settingsData";

/**
 * Settings › Notifications (NotificationSettingsView.swift): which job notifications post, and their sound. While the
 * system blocks the app's notifications, a row says so first, with a way to its settings page where there is one.
 */
export function NotificationsTab() {
  const settings = useNotificationSettings().data;
  // Read again when the window comes back, e.g. from the system settings with the switch flipped.
  const blocked = useQuery<BlockedBySystem_Serialize | null, AppError>({
    queryKey: [...queryKeys.settings.notifications, "blocked"],
    queryFn: () => commands.notificationsBlocked(),
    refetchOnWindowFocus: "always",
  }).data;
  const errors = useErrorAlert("Can't change this setting");
  const setPreferences = useSetPreferences(errors.show);
  if (!settings) return null;

  return (
    <>
      <SettingsSection
        title="Jobs"
        footer="System notifications. A paused job waits for you, e.g. for credits or a sign-in."
      >
        {blocked && (
          <SettingsRow label={blocked.label} detail={blocked.detail} isFirst>
            {blocked.openTitle !== null && (
              <Button
                variant="secondary"
                title={blocked.openTitle}
                onClick={() => void commands.notificationsOpenSystemSettings().catch(() => undefined)}
              />
            )}
          </SettingsRow>
        )}
        <SettingsRow label="Done" isFirst={!blocked}>
          <Switch
            label="Done"
            isOn={settings.onDone}
            onChange={(isOn) => {
              setPreferences({ "settings.notifyOnFinish": isOn });
            }}
          />
        </SettingsRow>
        <SettingsRow label="Failed, rejected or paused">
          <Switch
            label="Failed, rejected or paused"
            isOn={settings.onProblems}
            onChange={(isOn) => {
              setPreferences({ "settings.notifyOnProblems": isOn });
            }}
          />
        </SettingsRow>
        <SettingsRow label="Play sound" isDisabled={!settings.canPlaySound}>
          <Switch
            label="Play sound"
            isOn={settings.playsSound}
            disabled={!settings.canPlaySound}
            onChange={(isOn) => {
              setPreferences({ "settings.notificationSound": isOn });
            }}
          />
        </SettingsRow>
      </SettingsSection>
      {errors.alert}
    </>
  );
}
