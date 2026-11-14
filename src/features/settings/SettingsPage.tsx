import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import { useIsConnected } from "../../app/liveData";
import { PageHeader } from "../../design/Components";
import { FilterMenu, SegmentedControl } from "../../design/Pickers";
import { AgentsTab } from "./AgentsTab";
import { ConnectionTab } from "./ConnectionTab";
import { GeneralTab } from "./GeneralTab";
import { NotificationsTab } from "./NotificationsTab";
import { OutputTab } from "./OutputTab";
import { ShortcutsTab } from "./ShortcutsTab";
import { SpendingTab } from "./SpendingTab";
import { StorageTab } from "./StorageTab";
import { SubtitlesTab } from "./SubtitlesTab";

type SettingsTab =
  | "connection"
  | "general"
  | "output"
  | "subtitles"
  | "storage"
  | "spending"
  | "notifications"
  | "agents"
  | "shortcuts";

const allTabs: readonly { value: SettingsTab; title: string }[] = [
  { value: "connection", title: "Connection" },
  { value: "general", title: "General" },
  { value: "output", title: "Output" },
  { value: "subtitles", title: "Subtitles" },
  { value: "storage", title: "Storage" },
  { value: "spending", title: "Spending" },
  { value: "notifications", title: "Notifications" },
  { value: "agents", title: "Agents" },
  { value: "shortcuts", title: "Shortcuts" },
];

/** Whether the tab switcher fits beside the sidebar: a copy is measured off screen (`ViewThatFits`). */
function useTabsFit() {
  const row = useRef<HTMLDivElement>(null);
  const natural = useRef<HTMLDivElement>(null);
  const [fits, setFits] = useState(true);
  const measure = useCallback(() => {
    if (row.current && natural.current) setFits(natural.current.scrollWidth <= row.current.clientWidth);
  }, []);
  useLayoutEffect(measure, [measure]);
  useEffect(() => {
    if (typeof ResizeObserver === "undefined" || !row.current) return;
    const observer = new ResizeObserver(measure);
    observer.observe(row.current);
    return () => {
      observer.disconnect();
    };
  }, [measure]);
  return { row, natural, fits };
}

/** Settings (SettingsView.swift): the header and the tab switcher. The spending cap belongs to a connection, so
 *  Spending is listed only while connected. Nine tabs need more width than a narrow window has; then they are a menu.
 */
export function SettingsPage() {
  const isConnected = useIsConnected();
  const tabs = allTabs.filter((tab) => tab.value !== "spending" || isConnected);
  const [tab, setTab] = useState<SettingsTab>("connection");
  const shown = tabs.some((candidate) => candidate.value === tab) ? tab : "connection";
  const { row, natural, fits } = useTabsFit();
  return (
    <div className="flex min-h-full flex-col px-page pt-page-top pb-page">
      <PageHeader
        title="Settings"
        subtitle="Your Higgsfield connection, where results go and what you hear about."
      />
      <div ref={row} className="relative pb-x-large">
        <div ref={natural} aria-hidden inert className="invisible absolute flex w-max">
          <SegmentedControl isCompact value={shown} options={tabs} onChange={setTab} />
        </div>
        {fits ? (
          <SegmentedControl isCompact value={shown} options={tabs} onChange={setTab} />
        ) : (
          <FilterMenu title="Settings" value={shown} options={tabs} onChange={setTab} />
        )}
      </div>
      <div className="w-full max-w-640">
        {shown === "connection" && <ConnectionTab />}
        {shown === "general" && <GeneralTab />}
        {shown === "output" && <OutputTab />}
        {shown === "subtitles" && <SubtitlesTab />}
        {shown === "storage" && <StorageTab />}
        {shown === "spending" && <SpendingTab />}
        {shown === "notifications" && <NotificationsTab />}
        {shown === "shortcuts" && <ShortcutsTab />}
        {shown === "agents" && <AgentsTab />}
      </div>
    </div>
  );
}
