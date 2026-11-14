import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Channel } from "@tauri-apps/api/core";
import { Film, X } from "lucide-react";
import { useRef, useState } from "react";

import { commands, type ExportPreset } from "../../bindings";
import { Spinner } from "../../design/Spinner";
import { Button } from "../../design/Styles";
import { queryKeys } from "../../lib/queryKeys";
import { ContextMenu, type MenuItem } from "./LibraryDialogs";

/**
 * Export for Editing… on a result's page (`EditingExportButton.swift`): a preset, then a folder; the take is named
 * so nothing there is replaced, with its JSON sidecar and subtitles beside it, and shown in the file manager.
 */
export function EditingExportButton({ id }: { id: string }) {
  const client = useQueryClient();
  const button = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<{ x: number; y: number } | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const key = [...queryKeys.library.all, "editingExport", id];
  const menu = useQuery({ queryKey: key, queryFn: () => commands.editingExportMenu(id) }).data;
  if (!menu) return null;

  const refresh = () => void client.invalidateQueries({ queryKey: key });
  const exportWith = (preset: ExportPreset) => {
    const channel = new Channel<number | null>();
    channel.onmessage = (fraction) => {
      setProgress(fraction ?? 0);
    };
    setProgress(0);
    void commands.editingExport(id, preset, channel).finally(() => {
      setProgress(null);
      refresh();
    });
  };
  const toggle = (patch: { "export.writesSidecar"?: boolean; "export.writesSubtitles"?: boolean }) => {
    void commands.setPreferences(patch).then(refresh);
  };

  if (progress !== null || menu.isExporting) {
    return (
      <div className="flex items-center gap-6">
        <Spinner label={progress === null ? undefined : `${String(Math.round(progress * 100))}%`} />
        <Button variant="icon" icon={X} title="Cancel Export" hint="Cancel export" onClick={() => void commands.editingExportCancel()} />
      </div>
    );
  }
  const items: (MenuItem | "divider")[] = [
    ...menu.presets.map((item) => ({
      title: item.title,
      run: () => {
        exportWith(item.preset);
      },
    })),
    "divider",
    {
      title: "Include JSON Sidecar",
      isChecked: menu.writesSidecar,
      run: () => {
        toggle({ "export.writesSidecar": !menu.writesSidecar });
      },
    },
    ...(menu.hasSubtitles
      ? [
          {
            title: "Include Subtitles (SRT)",
            isChecked: menu.writesSubtitles,
            run: () => {
              toggle({ "export.writesSubtitles": !menu.writesSubtitles });
            },
          },
        ]
      : []),
  ];
  return (
    <>
      <div ref={button}>
        <Button
          variant="icon"
          icon={Film}
          title="Export for Editing…"
          hint="Export for editing"
          aria-haspopup="menu"
          disabled={!menu.isEnabled}
          onClick={() => {
            const rect = button.current?.getBoundingClientRect();
            setPosition({ x: rect?.left ?? 0, y: rect?.bottom ?? 0 });
          }}
        />
      </div>
      {position && (
        <ContextMenu
          x={position.x}
          y={position.y}
          items={items}
          onClose={() => {
            setPosition(null);
          }}
        />
      )}
    </>
  );
}
