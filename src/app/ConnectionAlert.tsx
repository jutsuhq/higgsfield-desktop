import { useNavigate } from "@tanstack/react-router";

import { commands, type UserMessage_Serialize } from "../bindings";
import { useCommandMutation } from "../lib/ipc";
import { Button } from "../design/Styles";
import { AlertDialog } from "./Dialog";
import { useConnection } from "./liveData";

/**
 * The connection's alert (MainWindow.swift `.alert`), over any page: an invalid key, a failed sign-in, a sign-out
 * that couldn't reach Higgsfield. Its action leads to Settings or the Higgsfield console; every button dismisses it.
 */
export function ConnectionAlert() {
  const alert = useConnection()?.alert;
  return alert ? <AlertBody alert={alert} /> : null;
}

function AlertBody({ alert }: { alert: UserMessage_Serialize }) {
  const navigate = useNavigate();
  const dismiss = useCommandMutation(() => commands.dismissConnectionAlert());
  const close = () => {
    dismiss.mutate();
  };
  return (
    <AlertDialog title={alert.title} message={alert.message} onCancel={close}>
      {alert.action === "openSettings" && (
        <Button
          variant="secondary"
          title="Open Settings"
          onClick={() => {
            close();
            void navigate({ to: "/settings" });
          }}
        />
      )}
      {alert.action === "openConsole" && (
        <Button
          variant="secondary"
          title="Open Console"
          onClick={() => {
            close();
            void commands.openHiggsfieldConsole();
          }}
        />
      )}
      <Button variant="secondary" title="OK" onClick={close} />
    </AlertDialog>
  );
}
