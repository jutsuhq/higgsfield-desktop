import { AlertDialog } from "../../app/Dialog";
import { commands, type SubtitleEngine } from "../../bindings";
import { SegmentedControl } from "../../design/Pickers";
import { Button } from "../../design/Styles";
import { SettingsRow, SettingsSection, SettingsValueRow } from "./SettingsRows";
import { useSubtitleSettings } from "./settingsData";

/**
 * Settings › Subtitles (SubtitleSettingsView.swift): the default speech model, and the Whisper model the app downloads
 * itself: Download, its progress with Cancel, then its size on disk with Remove. `SubtitlesChanged` brings it up to date.
 */
export function SubtitlesTab() {
  const settings = useSubtitleSettings().data;
  if (!settings) return null;
  const isDisabled = !settings.isAvailable;
  const state = settings.modelState;

  return (
    <div className="flex flex-col gap-x-large">
      <SettingsSection title="Subtitles">
        <SettingsRow label="Model" detail={settings.autoNote} isFirst isDisabled={isDisabled}>
          <fieldset disabled={isDisabled} className="m-0 border-0 p-0">
            <SegmentedControl
              isCompact
              value={settings.defaultEngine}
              options={settings.engines.map((option) => ({ value: option.value as SubtitleEngine, title: option.title }))}
              onChange={(engine) => void commands.subtitlesSetDefaultEngine(engine)}
            />
          </fieldset>
        </SettingsRow>
      </SettingsSection>
      <SettingsSection title={settings.modelsFolder} footer={settings.modelCredit}>
        <SettingsRow label={settings.modelName} detail={settings.modelSummary} isFirst isDisabled={isDisabled}>
          {state.kind === "notDownloaded" && (
            <Button
              variant="secondary"
              title="Download"
              disabled={isDisabled}
              hint="Downloads the model from Hugging Face into Jutsu Studio's own folder."
              onClick={() => void commands.subtitlesDownloadModel()}
            />
          )}
          {state.kind === "downloading" && (
            <span className="flex items-center gap-small">
              <progress aria-label="Download" value={state.fraction ?? undefined} max={1} className="w-[120px]" />
              <Button variant="secondary" title="Cancel" onClick={() => void commands.subtitlesCancelDownload()} />
            </span>
          )}
          {state.kind === "ready" && (
            <span className="flex items-center gap-small">
              {settings.modelSize !== null && <span className="text-body text-muted">{settings.modelSize}</span>}
              <Button variant="secondary" title="Remove" onClick={() => void commands.subtitlesRemoveModel()} />
            </span>
          )}
        </SettingsRow>
        <SettingsValueRow label="Details" value={settings.modelFacts} />
        <SettingsValueRow label="Languages" value={settings.modelLanguages} />
        <SettingsRow label="Source" isDisabled={isDisabled}>
          <span className="flex gap-small text-body text-muted">
            <span title={settings.modelUrl}>Hugging Face</span>
            <span title={settings.licenseUrl}>{settings.license}</span>
          </span>
        </SettingsRow>
      </SettingsSection>
      {settings.failure !== null && (
        <AlertDialog
          title={settings.failure.title}
          message={settings.failure.message}
          onCancel={() => void commands.subtitlesTakeFailure()}
        >
          <Button variant="primary" title="OK" onClick={() => void commands.subtitlesTakeFailure()} />
        </AlertDialog>
      )}
    </div>
  );
}
