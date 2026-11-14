import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { commands, type OnDeviceKind, type OnDeviceRunRow_Serialize } from "../../bindings";
import { AlertDialog, Dialog } from "../../app/Dialog";
import { RenderProgress, SectionLabel } from "../../design/Components";
import { SegmentedControl } from "../../design/Pickers";
import { Spinner } from "../../design/Spinner";
import { Button } from "../../design/Styles";
import { cx } from "../../design/cx";
import { useCommand, useCommandMutation } from "../../lib/ipc";
import { queryKeys } from "../../lib/queryKeys";
import { errorText } from "../create/createData";
import { useShownSheet } from "../../app/agentShow";

/**
 * The sheet of an Enhance run on this computer (`OnDeviceEnhanceSheet.swift`): its options, how long it takes and
 * that it is free. Before the engine's first download it asks to agree to it, naming its size. Starting adds the run;
 * the result page shows how far it is.
 */
export function OnDeviceSheet({
  id,
  kind,
  onClose,
}: {
  id: string;
  kind: OnDeviceKind;
  onClose: () => void;
}) {
  const [motion, setMotion] = useState<string | null>(null);
  const [agrees, setAgrees] = useState(false);
  const [refusal, setRefusal] = useState<string | null>(null);
  const [isBusy, setIsBusy] = useState(false);
  const client = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.onDevice.sheet(id, kind, motion),
    queryFn: () => commands.onDeviceSheet(id, kind, motion),
  });
  const sheet = query.data;
  const start = () => {
    if (!sheet) return;
    setIsBusy(true);
    void commands
      .onDeviceStart(id, kind, sheet.motion, agrees)
      .then((outcome) => {
        if (outcome.kind === "refused") {
          setRefusal(outcome.reason);
          return;
        }
        void client.invalidateQueries({ queryKey: queryKeys.library.all });
        onClose();
      })
      .catch((error: unknown) => {
        setRefusal(errorText(error));
      })
      .finally(() => {
        setIsBusy(false);
      });
  };
  return (
    <Dialog title={sheet?.title ?? "Enhance"} onCancel={onClose} className="w-480" sheet="onDevice">
      <div className="flex flex-col gap-6">
        <p className="m-0 text-heading font-bold">{sheet?.title ?? ""}</p>
        {sheet && <p className="m-0 truncate text-body text-muted">{sheet.itemTitle}</p>}
      </div>
      {!sheet ? (
        <Spinner />
      ) : (
        <>
          <div className="flex flex-col gap-small">
            {sheet.motions.length > 0 && sheet.motion !== null && (
              <>
                <SectionLabel title="Motion" />
                <SegmentedControl
                  isCompact
                  value={sheet.motion}
                  options={sheet.motions}
                  onChange={(value) => {
                    setMotion(value);
                  }}
                />
              </>
            )}
            {sheet.sizes !== null && (
              <>
                <SectionLabel title="Size" />
                <p className="m-0 text-body font-semibold tabular-nums">{sheet.sizes}</p>
              </>
            )}
            {sheet.optionNotes.map((note) => (
              <p
                key={note.text}
                className={cx("m-0 text-caption", note.isWarning ? "text-warning" : "text-muted")}
              >
                {note.text}
              </p>
            ))}
          </div>
          {sheet.modelNotice !== null && (
            <div className="flex flex-col gap-small">
              <p className="m-0 text-caption text-muted">{sheet.modelNotice}</p>
              <label className="flex items-center gap-6 text-body">
                <input
                  type="checkbox"
                  checked={agrees}
                  onChange={(event) => {
                    setAgrees(event.target.checked);
                  }}
                />
                {sheet.consentTitle}
              </label>
            </div>
          )}
          <div className="flex flex-col gap-6">
            {sheet.notes.map((note) => (
              <p
                key={note.text}
                className={cx("m-0 text-caption", note.isWarning ? "text-warning" : "text-muted")}
              >
                {note.text}
              </p>
            ))}
            {sheet.unavailableReason !== null && (
              <p className="m-0 text-caption text-warning">{sheet.unavailableReason}</p>
            )}
          </div>
          <div className="flex justify-end gap-small">
            <Button variant="secondary" title="Cancel" onClick={onClose} />
            <Button
              variant="primary"
              title={sheet.startTitle}
              price={sheet.priceTag}
              disabled={!sheet.canStart || isBusy || (sheet.modelNotice !== null && !agrees)}
              onClick={start}
            />
          </div>
        </>
      )}
      {refusal !== null && (
        <AlertDialog
          title="Can't start this"
          message={refusal}
          onCancel={() => {
            setRefusal(null);
          }}
        >
          <Button
            variant="secondary"
            title="OK"
            onClick={() => {
              setRefusal(null);
            }}
          />
        </AlertDialog>
      )}
    </Dialog>
  );
}

/** How far the run on the item `id` is, with its Cancel; nothing while none waits or runs on it. */
export function OnDeviceRunStatus({ id }: { id: string }) {
  const runs = useCommand(queryKeys.onDevice.runs, commands.onDeviceRuns);
  const cancel = useCommandMutation(
    (runId: string) => commands.onDeviceCancel(runId),
    [queryKeys.onDevice.all, queryKeys.library.all],
  );
  const run: OnDeviceRunRow_Serialize | undefined = runs.data?.find((row) => row.itemId === id);
  if (!run) return null;
  return (
    // `OnDeviceRunRow`: the bar (a spinner while it waits), the progress text, and Cancel as a link.
    <div className="flex items-center gap-small" role="status" aria-label={run.progress}>
      {run.fraction === null ? (
        <Spinner />
      ) : (
        <div className="min-w-60 flex-1">
          <RenderProgress value={run.fraction} />
        </div>
      )}
      <p className="m-0 shrink-0 truncate text-small text-muted">{run.progress}</p>
      <Button
        variant="textLink"
        title="Cancel"
        hint={`Stop ${run.title}`}
        onClick={() => {
          cancel.mutate(run.id);
        }}
      />
    </div>
  );
}

/** Upscale on This Computer's sheet, when an agent's `show` asked for it on the item `id`. */
export function AgentShownOnDeviceSheet({ id }: { id: string }) {
  const { item, sheet, set } = useShownSheet();
  if (item !== id || sheet !== "upscale_on_mac") return null;
  return (
    <OnDeviceSheet
      id={id}
      kind="upscaleVideo"
      onClose={() => {
        set(null, null);
      }}
    />
  );
}
