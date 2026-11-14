import { TriangleAlert } from "lucide-react";
import { useState } from "react";

import { AlertDialog } from "../../app/Dialog";
import { useConnection, useConnectionDetails } from "../../app/liveData";
import { commands, type SignOutConfirmation } from "../../bindings";
import { Button } from "../../design/Styles";
import { useCommandMutation } from "../../lib/ipc";
import { queryKeys } from "../../lib/queryKeys";
import { SettingsRow, SettingsSection, SettingsValueRow, Switch } from "./SettingsRows";

/** Settings › Connection (ConnectionSettingsView.swift): the current connection, with Sign Out or Remove. */
export function ConnectionTab() {
  const connection = useConnection();
  const details = useConnectionDetails().data;
  const [confirming, setConfirming] = useState<SignOutConfirmation | null>(null);
  const signIn = useCommandMutation(() => commands.startSignIn());
  const signOut = useCommandMutation(() => commands.signOut());
  const setUnlimited = useCommandMutation(
    (isOn: boolean) => commands.setPreferences({ "settings.useUnlimited": isOn }),
    [queryKeys.connection.details],
  );
  const isAccount = connection?.kind === "account";
  const rows = connection?.kind != null ? (details?.rows ?? []) : [];

  return (
    <div className="flex flex-col gap-large">
      <SettingsSection
        title="Higgsfield"
        footer="One connection at a time. To switch, sign out and connect the other one."
      >
        {connection?.kind == null ? (
          <SettingsValueRow label="Not connected" value="Connect from the main window" isFirst />
        ) : (
          rows.map((row, index) => (
            <SettingsValueRow
              key={row.label}
              label={row.label}
              value={row.value}
              isMonospaced={row.isMonospaced}
              isFirst={index === 0}
            />
          ))
        )}
      </SettingsSection>

      {isAccount && details?.unlimitedText != null && (
        <SettingsSection
          title="Unlimited generations"
          footer="Your plan has unlimited generations for these models right now. When this is on, their jobs use them instead of credits and don't count toward the daily cap. If they run out, Higgsfield charges credits as usual."
        >
          <SettingsRow label="Use unlimited generations" detail={details.unlimitedText} isFirst>
            <Switch
              label="Use unlimited generations"
              isOn={details.usesUnlimited}
              onChange={(isOn) => {
                setUnlimited.mutate(isOn);
              }}
            />
          </SettingsRow>
        </SettingsSection>
      )}

      {connection?.needsAttention && (
        <p className="m-0 flex items-center gap-6 text-[12.5px] text-danger">
          <TriangleAlert aria-hidden size={13} className="shrink-0" />
          Higgsfield refused this connection. Sign in again or connect another key.
        </p>
      )}

      {connection?.signOut && (
        <div className="flex items-center">
          {isAccount && (
            <Button
              variant="secondary"
              title="Sign In Again"
              onClick={() => {
                signIn.mutate();
              }}
            />
          )}
          <span className="flex-1" />
          <Button
            variant="secondary"
            destructive
            title={connection.signOut.confirmTitle}
            onClick={() => {
              setConfirming(connection.signOut);
            }}
          />
        </div>
      )}

      {confirming && (
        <AlertDialog
          title={confirming.title}
          message={confirming.message}
          onCancel={() => {
            setConfirming(null);
          }}
        >
          <Button
            variant="secondary"
            title="Cancel"
            onClick={() => {
              setConfirming(null);
            }}
          />
          <Button
            variant="secondary"
            destructive
            title={confirming.confirmTitle}
            onClick={() => {
              setConfirming(null);
              signOut.mutate();
            }}
          />
        </AlertDialog>
      )}
    </div>
  );
}
