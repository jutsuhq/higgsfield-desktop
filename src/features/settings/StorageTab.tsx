import { useState } from "react";

import { commands, type Confirmation } from "../../bindings";
import { SegmentedControl } from "../../design/Pickers";
import { Button } from "../../design/Styles";
import { useCommand, useCommandMutation } from "../../lib/ipc";
import { queryKeys } from "../../lib/queryKeys";
import { SettingsRow, SettingsSection, SettingsValueRow } from "./SettingsRows";
import { ConfirmDialog, useErrorAlert } from "./settingsData";

const measuring = "Measuring…";

/**
 * Settings › Storage (StorageSettingsView.swift): disk use of results, prepared sources, preset previews and downloaded
 * inputs, measured in Rust off the main thread, with Clear Cache and moving old drafts to the trash.
 */
export function StorageTab() {
  const usage = useCommand(queryKeys.settings.storageSummary, () => commands.storageSummary()).data;
  // `null` until picked: Rust's default (30 days).
  const [days, setDays] = useState<number | null>(null);
  const drafts = useCommand(queryKeys.settings.oldDrafts(days), () => commands.oldDrafts(days)).data;
  const errors = useErrorAlert("Can't free the space");
  const [isConfirmingClear, setConfirmingClear] = useState(false);
  /** The drafts the open confirmation counted; exactly these move. */
  const [confirming, setConfirming] = useState<{
    confirmation: Confirmation;
    days: number;
    jobIds: string[];
  } | null>(null);
  const clearCache = useCommandMutation(() => commands.clearCache(), [queryKeys.settings.storage]);
  const trash = useCommandMutation(
    ({ days, jobIds }: { days: number; jobIds: string[] }) => commands.trashOldDrafts(days, jobIds),
    [queryKeys.settings.storage],
  );

  return (
    <div className="flex flex-col gap-x-large">
      <SettingsSection title="Disk use" footer="Results count only files of jobs in the library.">
        <SettingsValueRow label="Results" value={usage?.results ?? measuring} isFirst />
        <SettingsValueRow label="Prepared sources" value={usage?.sources ?? measuring} />
        <SettingsValueRow label="Preset previews" value={usage?.cache ?? measuring} />
        <SettingsValueRow label="Motion Library and uploads" value={usage?.media ?? measuring} />
        <SettingsValueRow label="Speech models" value={usage?.speechModels ?? measuring} />
      </SettingsSection>

      <SettingsSection
        title="Cache"
        footer={
          usage?.isBusy
            ? "Available when the queue and image requests are done: they still use their prepared files."
            : undefined
        }
      >
        <SettingsRow label="Clear cache" detail={usage?.cacheDetail ?? measuring} isFirst>
          <Button
            variant="secondary"
            title="Clear Cache"
            disabled={!usage?.canClearCache}
            onClick={() => {
              setConfirmingClear(true);
            }}
          />
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        title="Old drafts"
        footer="Draft results go to the Trash. The library keeps their jobs, inputs and costs."
      >
        <SettingsRow label="Older than" isFirst>
          {drafts && (
            <SegmentedControl<number>
              isCompact
              value={drafts.days}
              options={drafts.dayOptions.map((value) => ({ value, title: `${String(value)} days` }))}
              onChange={setDays}
            />
          )}
        </SettingsRow>
        <SettingsRow label="Remove drafts" detail={drafts?.summary ?? measuring}>
          <Button
            variant="secondary"
            title="Move to Trash…"
            disabled={!drafts?.confirmation}
            onClick={() => {
              if (drafts?.confirmation) {
                setConfirming({
                  confirmation: drafts.confirmation,
                  days: drafts.days,
                  jobIds: drafts.jobIds,
                });
              }
            }}
          />
        </SettingsRow>
      </SettingsSection>

      {isConfirmingClear && (
        <ConfirmDialog
          title="Clear the cache?"
          message="Prepared sources, preset previews and downloaded Motion Library clips and uploads are deleted. The library, templates, the files in the Create form and your results stay."
          action="Clear Cache"
          onCancel={() => {
            setConfirmingClear(false);
          }}
          onConfirm={() => {
            clearCache.mutate(undefined, { onError: errors.show });
          }}
        />
      )}
      {confirming && (
        <ConfirmDialog
          title={confirming.confirmation.title}
          message={confirming.confirmation.message}
          action="Move to Trash"
          onCancel={() => {
            setConfirming(null);
          }}
          onConfirm={() => {
            trash.mutate({ days: confirming.days, jobIds: confirming.jobIds }, { onError: errors.show });
          }}
        />
      )}
      {errors.alert}
    </div>
  );
}
