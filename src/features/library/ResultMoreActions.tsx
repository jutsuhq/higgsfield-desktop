import { useState, type ReactNode } from "react";

import { commands, type Notice, type QueueConfirmation } from "../../bindings";
import { AlertDialog } from "../../app/Dialog";
import { useIsConnected } from "../../app/liveData";
import { useToastStore } from "../../app/toastStore";
import { Button } from "../../design/Styles";
import { NameDialog, type Naming } from "../create/TemplatesMenu";

/**
 * Duplicate on a result's More menu (`LibraryDetail.moreMenu`): its request is queued again, as the Queue's Duplicate
 * does, with the same question over today's cap. Off without a connection and while one is being queued.
 */
export function useDuplicate(id: string): { run: () => void; isDisabled: boolean; dialog: ReactNode } {
  const isConnected = useIsConnected();
  const showQueued = useToastStore((state) => state.showQueued);
  const [isBusy, setIsBusy] = useState(false);
  const [confirmation, setConfirmation] = useState<QueueConfirmation | null>(null);

  const duplicate = (overCapApproved: boolean) => {
    setIsBusy(true);
    commands
      .queueDuplicate(id, overCapApproved)
      .then((outcome) => {
        if (outcome.kind === "queued") showQueued(outcome.notice);
        if (outcome.kind === "confirm") setConfirmation(outcome.confirmation);
      })
      .catch(() => undefined)
      .finally(() => {
        setIsBusy(false);
      });
  };

  const dialog = confirmation && (
    <AlertDialog
      title={confirmation.title}
      message={confirmation.message}
      onCancel={() => {
        setConfirmation(null);
      }}
    >
      <Button
        variant="secondary"
        title="Cancel"
        onClick={() => {
          setConfirmation(null);
        }}
      />
      <Button
        variant="primary"
        title={confirmation.confirmTitle}
        onClick={() => {
          setConfirmation(null);
          duplicate(true);
        }}
      />
    </AlertDialog>
  );
  return {
    run: () => {
      duplicate(false);
    },
    isDisabled: !isConnected || isBusy || confirmation !== null,
    dialog,
  };
}

/**
 * Save as Template… on a result's More menu: the name starts as Rust suggests it, and the result's inputs (a draft's
 * at its final resolution) become a template Create offers.
 */
export function useSaveTemplate(id: string): { run: () => void; dialog: ReactNode } {
  const [naming, setNaming] = useState<Naming | null>(null);
  const [failure, setFailure] = useState<Notice | null>(null);

  const save = (name: string) => {
    setNaming(null);
    commands
      .librarySaveTemplate(id, name)
      .then((outcome) => {
        if (outcome.kind === "failed") setFailure(outcome.notice);
      })
      .catch(() => undefined);
  };

  const dialog = (
    <>
      {naming && (
        <NameDialog
          naming={naming}
          onCancel={() => {
            setNaming(null);
          }}
          onConfirm={save}
        />
      )}
      {failure && (
        <AlertDialog
          title={failure.title}
          message={failure.detail}
          onCancel={() => {
            setFailure(null);
          }}
        >
          <Button
            variant="primary"
            title="OK"
            onClick={() => {
              setFailure(null);
            }}
          />
        </AlertDialog>
      )}
    </>
  );
  return {
    run: () => {
      commands
        .libraryTemplateName(id)
        .then((name) => {
          setNaming({ title: "Save as Template", confirmTitle: "Save", name, renaming: null });
        })
        .catch(() => undefined);
    },
    dialog,
  };
}
