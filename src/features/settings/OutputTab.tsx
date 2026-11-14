import { useEffect, useRef, useState } from "react";

import { commands, type OutputNameTemplate, type Resolution } from "../../bindings";
import { cx } from "../../design/cx";
import { HoverHint } from "../../design/HoverHint";
import { SegmentedControl } from "../../design/Pickers";
import { Button, focusRing, TextField } from "../../design/Styles";
import { ValueStepper } from "../../design/ValueStepper";
import { useCommand, useCommandMutation } from "../../lib/ipc";
import { queryKeys } from "../../lib/queryKeys";
import { SettingsRow, SettingsSection } from "./SettingsRows";
import { useErrorAlert, useOutputSettings, useSetPreferences } from "./settingsData";

/**
 * Settings › Output (OutputSettingsView.swift): where results download, the resolutions, parallel jobs and the file
 * names of results with a live example.
 */
export function OutputTab() {
  const settings = useOutputSettings().data;
  const errors = useErrorAlert("Can't change this setting");
  const setPreferences = useSetPreferences(errors.show);
  const choose = useCommandMutation(() => commands.chooseOutputFolder());
  const useDefault = useCommandMutation(() => commands.resetOutputFolder());
  if (!settings) return null;
  const resolutions = settings.resolutions.map((value) => ({ value, title: value }));

  return (
    <div className="flex flex-col gap-x-large">
      <SettingsSection title="Results" footer="Results download here automatically when a job finishes.">
        <SettingsRow label="Folder" detail={settings.folderDisplay} isFirst>
          <div className="flex items-center gap-small">
            {settings.isCustomFolder && (
              <Button
                variant="secondary"
                title="Use Default"
                onClick={() => {
                  useDefault.mutate(undefined, { onError: errors.show });
                }}
              />
            )}
            <Button
              variant="secondary"
              title="Change…"
              onClick={() => {
                choose.mutate(undefined, { onError: errors.show });
              }}
            />
          </div>
        </SettingsRow>
        <SettingsRow label="Default resolution">
          <SegmentedControl<Resolution>
            isCompact
            value={settings.resolution}
            options={resolutions}
            onChange={(resolution) => {
              setPreferences({ "settings.defaultResolution": resolution });
            }}
          />
        </SettingsRow>
        <SettingsRow label="Draft resolution">
          <SegmentedControl<Resolution>
            isCompact
            value={settings.draftResolution}
            options={resolutions}
            onChange={(resolution) => {
              setPreferences({ "settings.draftResolution": resolution });
            }}
          />
        </SettingsRow>
        <SettingsRow label="Parallel jobs" detail={settings.concurrencyDetail}>
          <ValueStepper
            label="Parallel jobs"
            value={settings.maxConcurrency}
            min={settings.minConcurrency}
            max={settings.maxConcurrencyLimit}
            onChange={(limit) => {
              setPreferences({ "settings.maxConcurrency": limit });
            }}
          />
        </SettingsRow>
      </SettingsSection>

      <FileNames
        stored={settings.namePattern}
        templates={settings.templates}
        tokenHelp={settings.tokenHelp}
        onChange={(pattern) => {
          setPreferences({ "settings.outputNamePattern": pattern });
        }}
      />
      {errors.alert}
    </div>
  );
}

/** The File names section: the pattern, typed or picked from Templates, and the name it gives now. */
function FileNames({
  stored,
  templates,
  tokenHelp,
  onChange,
}: {
  stored: string;
  templates: OutputNameTemplate[];
  tokenHelp: string;
  onChange: (pattern: string) => void;
}) {
  // Typing is kept here, so a round trip through Rust never moves the caret.
  const [pattern, setPattern] = useState(stored);
  const offset = -new Date().getTimezoneOffset();
  const preview = useCommand(queryKeys.settings.outputName(pattern), () =>
    commands.outputNamePreview(pattern, offset),
  ).data;
  const change = (next: string) => {
    setPattern(next);
    onChange(next);
  };

  return (
    <SettingsSection title="File names" footer={preview?.footer}>
      <SettingsRow label="Pattern" isFirst>
        <div className="flex items-center gap-small">
          <TemplatesMenu templates={templates} onPick={change} />
          <HoverHint text={tokenHelp}>
            <TextField
              aria-label="Pattern"
              placeholder="Pattern"
              value={pattern}
              spellCheck={false}
              className="w-240 font-mono"
              onChange={(event) => {
                change(event.target.value);
              }}
            />
          </HoverHint>
        </div>
      </SettingsRow>
      <SettingsRow label="Example">
        <span className="min-w-0 truncate font-mono text-body text-muted select-text">
          {preview?.example}
        </span>
      </SettingsRow>
    </SettingsSection>
  );
}

/** `Menu("Templates")`: the ready-made patterns. */
function TemplatesMenu({
  templates,
  onPick,
}: {
  templates: OutputNameTemplate[];
  onPick: (pattern: string) => void;
}) {
  const [isOpen, setOpen] = useState(false);
  const container = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!isOpen) return;
    const close = (event: Event) => {
      if (event instanceof KeyboardEvent && event.key !== "Escape") return;
      if (event instanceof MouseEvent && container.current?.contains(event.target as Node)) return;
      setOpen(false);
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", close);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", close);
    };
  }, [isOpen]);

  return (
    <div ref={container} className="relative">
      <Button
        variant="secondary"
        title="Templates"
        aria-haspopup="menu"
        aria-expanded={isOpen}
        onClick={() => {
          setOpen(!isOpen);
        }}
      />
      {isOpen && (
        <div
          role="menu"
          className="absolute top-full right-0 z-40 mt-6 flex min-w-max flex-col rounded-field border border-stroke bg-card p-3"
        >
          {templates.map((template) => (
            <button
              key={template.pattern}
              type="button"
              role="menuitem"
              className={cx(
                "cursor-pointer rounded-field px-10 py-6 text-left text-body text-text hovered:bg-glass",
                focusRing,
              )}
              onClick={() => {
                setOpen(false);
                onPick(template.pattern);
              }}
            >
              {template.title}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
