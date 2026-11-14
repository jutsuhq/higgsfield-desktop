import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ChevronRight,
  CircleAlert,
  CircleX,
  Film,
  Folder,
  Power,
  Settings,
  Sparkles,
  TriangleAlert,
} from "lucide-react";
import { useCallback, useEffect, useState, type ReactNode } from "react";

import {
  commands,
  events,
  type AppError,
  type TrayOpen,
  type TrayPanel_Serialize,
  type TrayRecent_Serialize,
  type TrayRunning_Serialize,
  type TrayToday_Serialize,
} from "../../bindings";
import appIcon from "../../design/assets/app-icon.png";
import { RenderProgress } from "../../design/Components";
import { cx } from "../../design/cx";
import { HoverHint } from "../../design/HoverHint";
import { Button, focusRing } from "../../design/Styles";
import { useInvalidateOn } from "../../lib/events";
import { queryKeys } from "../../lib/queryKeys";
import { useThumbnail } from "../library/libraryData";
import { ContextMenu, type MenuItem } from "../library/LibraryDialogs";
import { Switch } from "../settings/SettingsRows";

/** How often the panel asks again while it shows, so elapsed times, estimates and the today bar move. */
const refreshInterval = 1000;

/**
 * Whether the panel shows: it hides as soon as it loses focus, so focus and the page's visibility say it. Asks again
 * at once when it shows again (`TimelineView(.periodic(from: .now, by: 1))` while `isOpen`).
 */
function usePanelShown(onShow: () => void) {
  const [isShown, setShown] = useState(() => document.visibilityState !== "hidden");
  useEffect(() => {
    const show = () => {
      setShown(true);
      onShow();
    };
    const hide = () => {
      setShown(false);
    };
    const visibility = () => {
      if (document.visibilityState === "hidden") hide();
      else show();
    };
    window.addEventListener("focus", show);
    window.addEventListener("blur", hide);
    document.addEventListener("visibilitychange", visibility);
    return () => {
      window.removeEventListener("focus", show);
      window.removeEventListener("blur", hide);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, [onShow]);
  return isShown;
}

/** The panel is as tall as what it shows, up to its most; Rust keeps it by the icon (`MenuBarContent`'s fitted height). */
function useFitPanel(content: HTMLElement | null) {
  useEffect(() => {
    if (!content || typeof ResizeObserver === "undefined") return;
    const fit = () => {
      void commands.trayFit(content.offsetHeight);
    };
    const observer = new ResizeObserver(fit);
    observer.observe(content);
    return () => {
      observer.disconnect();
    };
  }, [content]);
}

/**
 * The tray panel (MenuBarContent.swift, MenuBarSections.swift): the connection and the agents, today's agent budget,
 * what waits for the person, what runs and the last results, then Open, Settings and Quit. Each section is a title over
 * a quiet card of rows. Everything it shows comes from Rust's `tray_panel`; each row asks Rust to act.
 */
export function TrayPanel() {
  const queryClient = useQueryClient();
  const isShown = usePanelShown(
    useCallback(() => {
      void queryClient.refetchQueries({ queryKey: queryKeys.tray });
    }, [queryClient]),
  );
  const panel = useQuery<TrayPanel_Serialize, AppError>({
    queryKey: queryKeys.tray,
    queryFn: () => commands.trayPanel(),
    refetchInterval: isShown ? refreshInterval : false,
  }).data;
  useInvalidateOn(events.queueChanged, [queryKeys.tray]);
  useInvalidateOn(events.connectionChanged, [queryKeys.tray]);
  useInvalidateOn(events.preferencesChanged, [queryKeys.tray]);
  useInvalidateOn(events.libraryChanged, [queryKeys.tray]);

  const [content, setContent] = useState<HTMLElement | null>(null);
  useFitPanel(content);
  useEffect(() => {
    // Escape closes the panel, as it closes a menu bar panel.
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) void commands.trayClose();
    };
    window.addEventListener("keydown", close);
    return () => {
      window.removeEventListener("keydown", close);
    };
  }, []);

  const open = (place: TrayOpen) => {
    void commands.trayOpen(place);
  };

  return (
    <main className="h-screen overflow-y-auto bg-background text-text">
      <div ref={setContent} className="flex flex-col gap-14 p-14">
        {panel && <Header header={panel.header} />}

        {panel?.today && <Today today={panel.today} />}

        {panel && panel.needsYou.length > 0 && (
          <Section title="Needs you" overflow={panel.needsYouOverflow}>
            {panel.needsYou.map((item, index) => (
              <Row key={item.id} isFirst={index === 0}>
                <RowButton
                  hint={item.hint}
                  onClick={() => {
                    open({ kind: "attention", target: item.target });
                  }}
                >
                  <span className="flex items-center gap-10">
                    <CircleAlert aria-hidden size={14} className="shrink-0 fill-warning text-background" />
                    <TitleAndDetail title={item.title} detail={item.reason} detailLines={2} />
                    <ChevronRight
                      aria-hidden
                      size={11}
                      strokeWidth={3}
                      className="ml-x-small shrink-0 text-faint"
                    />
                  </span>
                </RowButton>
              </Row>
            ))}
          </Section>
        )}

        {panel && panel.running.length > 0 && (
          <Section title="Running" overflow={panel.runningOverflow}>
            {panel.running.map((item, index) => (
              <Row key={item.id} isFirst={index === 0}>
                <RunningRow
                  item={item}
                  onOpen={() => {
                    open({ kind: "queueJob", id: item.id });
                  }}
                />
              </Row>
            ))}
          </Section>
        )}

        {panel && panel.recent.length > 0 && (
          <Section title="Recent">
            {panel.recent.map((item, index) => (
              <Row key={item.id} isFirst={index === 0}>
                <RecentRow
                  item={item}
                  onOpen={() => {
                    open({ kind: "libraryJob", id: item.id });
                  }}
                />
              </Row>
            ))}
          </Section>
        )}

        {panel?.isIdle && <p className="m-0 py-6 text-center text-small text-muted">Nothing running</p>}

        <footer className="flex items-center gap-small">
          <Button
            variant="primary"
            title="Open Jutsu Studio"
            onClick={() => {
              open({ kind: "app" });
            }}
          />
          <span className="mr-auto" />
          <Button
            variant="icon"
            icon={Settings}
            title="Settings…"
            hint="Settings…"
            onClick={() => {
              open({ kind: "settings" });
            }}
          />
          <Button
            variant="icon"
            icon={Power}
            title="Quit Jutsu Studio"
            hint="Quit Jutsu Studio"
            onClick={() => {
              void commands.trayQuit();
            }}
          />
        </footer>
      </div>
    </main>
  );
}

/** The app, its connection and the agents: whether they are allowed, who sent requests lately, Pause agents. */
function Header({ header }: { header: TrayPanel_Serialize["header"] }) {
  const { agents } = header;
  const pauseHint = agents.control === "pause" ? "Pause the agents' jobs" : "Resume the agents' jobs";
  return (
    <header className="flex flex-col gap-10">
      <div className="flex items-center gap-10">
        {/* The app icon carries the signature gradient: the panel's one brand moment. */}
        <img src={appIcon} alt="" aria-hidden className="size-30 shrink-0 rounded-app-icon" />
        <div className="flex min-w-0 flex-col gap-1">
          <h1 className="m-0 text-body font-bold">Jutsu Studio</h1>
          <p className="m-0 truncate text-tiny text-muted tabular-nums">{header.connectionLine}</p>
        </div>
      </div>
      {header.warning !== null && (
        <p className="m-0 flex items-center gap-x-small text-tiny text-warning tabular-nums">
          <TriangleAlert aria-hidden size={11} className="shrink-0 fill-warning text-background" />
          {header.warning}
        </p>
      )}
      <PanelCard>
        <div className="flex items-center gap-10 px-10 py-small">
          <Sparkles
            aria-hidden
            size={13}
            strokeWidth={2.5}
            className={cx("w-18 shrink-0", agents.isAllowed ? "text-text" : "text-faint")}
          />
          <div className="mr-auto flex min-w-0 flex-col gap-1">
            <span className="text-small font-medium">Agents</span>
            <span className="truncate text-tiny text-muted">{agents.status}</span>
          </div>
          {agents.control !== null && (
            <Button
              variant="secondary"
              className="min-h-26 px-10 text-small"
              title={agents.control === "pause" ? "Pause" : "Resume"}
              aria-label={pauseHint}
              hint={pauseHint}
              onClick={() => {
                if (agents.control !== null) void commands.trayAgents(agents.control);
              }}
            />
          )}
          <Switch
            label="Allow agents"
            isOn={agents.isAllowed}
            onChange={(isOn) => {
              void commands.agentsSetAllowed(isOn);
            }}
          />
        </div>
      </PanelCard>
    </header>
  );
}

/** A card on the panel: a quiet fill with rounded corners (`PanelCard`). */
function PanelCard({ children }: { children: ReactNode }) {
  return <div className="flex flex-col overflow-hidden rounded-tile bg-card">{children}</div>;
}

/** A section's title over its card, with "and 2 more" under it (`PanelSection`). */
function Section({
  title,
  overflow,
  children,
}: {
  title: string;
  overflow?: string | null;
  children: ReactNode;
}) {
  return (
    <section aria-label={title} className="flex flex-col gap-6">
      <h2 className="m-0 pl-x-small text-tiny font-semibold text-muted">{title}</h2>
      <PanelCard>{children}</PanelCard>
      {overflow != null && <span className="pl-x-small text-tiny text-faint">{overflow}</span>}
    </section>
  );
}

/** A row of a card, with a divider above unless it is first (`PanelDivider`). */
function Row({ isFirst, children }: { isFirst: boolean; children: ReactNode }) {
  return (
    <div className="flex flex-col">
      {!isFirst && <div aria-hidden className="ml-10 h-px bg-stroke" />}
      {children}
    </div>
  );
}

/** A whole row that opens something, lit on hover as a menu item is (`PanelRowButton`). */
function RowButton({ hint, onClick, children }: { hint: string; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      title={hint}
      onClick={onClick}
      className={cx(
        "flex min-w-0 flex-1 flex-col px-10 py-7 text-left text-text",
        "enabled:cursor-pointer hovered:bg-well",
        focusRing,
      )}
    >
      {children}
    </button>
  );
}

function TitleAndDetail({
  title,
  detail,
  detailLines = 1,
}: {
  title: string;
  detail: string;
  detailLines?: 1 | 2;
}) {
  return (
    <span className="flex min-w-0 flex-1 flex-col gap-1">
      <span className="truncate text-small">{title}</span>
      <span
        className={cx("text-tiny text-muted tabular-nums", detailLines === 1 ? "truncate" : "line-clamp-2")}
      >
        {detail}
      </span>
    </span>
  );
}

/** Today's agent budget: what is spent and reserved on a thin bar, what is left, and the person's own cap. */
function Today({ today }: { today: TrayToday_Serialize }) {
  const spent = today.spentFraction ?? 0;
  const reserved = today.reservedFraction ?? 0;
  return (
    <Section title="Today">
      <div className="flex flex-col gap-7 px-10 py-9 whitespace-nowrap tabular-nums">
        <div className="flex items-baseline gap-small">
          <span className="mr-auto text-small font-medium">{today.spent}</span>
          {today.limit !== null && <span className="text-tiny text-muted">{today.limit}</span>}
        </div>
        <div
          role="meter"
          aria-label="Today's agent budget"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(spent * 100)}
          className="flex h-4 w-full overflow-hidden rounded-full bg-track"
        >
          <div className="h-full bg-accent" style={{ width: `${String(spent * 100)}%` }} />
          <div className="h-full bg-accent-estimate" style={{ width: `${String(reserved * 100)}%` }} />
        </div>
        <div className="flex items-baseline gap-small text-tiny text-muted">
          <span className="mr-auto">{today.reserved}</span>
          <span className={today.isUsedUp ? "text-warning" : undefined}>{today.remaining}</span>
        </div>
        {today.cap !== null && <span className="text-tiny text-faint">{today.cap}</span>}
      </div>
    </Section>
  );
}

/** Paid jobs and runs on this computer, with progress and Cancel. A run has no Queue row to open, so it isn't a button. */
function RunningRow({ item, onOpen }: { item: TrayRunning_Serialize; onOpen: () => void }) {
  const body = (
    <span className="flex w-full flex-col gap-5">
      <TitleAndDetail title={item.title} detail={item.detail} />
      {item.progress !== null ? (
        <RenderProgress value={item.progress} />
      ) : (
        item.estimate !== null && (
          <RenderProgress value={item.estimate.fraction ?? 0} estimate={item.estimate.label} />
        )
      )}
    </span>
  );
  return (
    <div className="flex items-center gap-small">
      {item.kind === "job" ? (
        <RowButton hint="Show in the Queue" onClick={onOpen}>
          {body}
        </RowButton>
      ) : (
        <div className="flex min-w-0 flex-1 px-10 py-7">{body}</div>
      )}
      {item.cancel !== null && (
        <HoverHint text={item.cancel.help} className="mr-small flex">
          <button
            type="button"
            aria-label={`Cancel ${item.title}`}
            disabled={!item.cancel.isEnabled}
            onClick={() => {
              void commands.trayCancel(item.id);
            }}
            className={cx(
              "flex rounded-full text-muted enabled:cursor-pointer hovered:text-text disabled:opacity-40",
              focusRing,
            )}
          >
            <CircleX aria-hidden size={15} />
          </button>
        </HoverHint>
      )}
    </div>
  );
}

/** A result's frame, small: the Library's still frame, a film symbol until then or without one (`PanelThumbnail`). */
function PanelThumbnail({ id, hasFile }: { id: string; hasFile: boolean }) {
  const url = useThumbnail(id, hasFile).data ?? null;
  return (
    <span className="flex h-27 w-48 shrink-0 items-center justify-center overflow-hidden rounded-thumb bg-well">
      {url === null ? (
        <Film aria-hidden size={11} className="text-faint" />
      ) : (
        <img src={url} alt="" aria-hidden className="size-full object-cover" />
      )}
    </span>
  );
}

/** A finished result: a click opens it in Library, the folder shows its file; both are on its context menu too. */
function RecentRow({ item, onOpen }: { item: TrayRecent_Serialize; onOpen: () => void }) {
  const file = item.file;
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  const reveal = () => {
    if (file !== null) void commands.trayReveal(file);
  };
  const menu: MenuItem[] = [{ title: "Show in Library", run: onOpen }];
  if (file !== null) menu.push({ title: "Show in File Manager", run: reveal });
  return (
    <div
      className="flex items-center gap-small"
      onContextMenu={(event) => {
        event.preventDefault();
        setMenuAt({ x: event.clientX, y: event.clientY });
      }}
    >
      <RowButton hint="Show in Library" onClick={onOpen}>
        <span className="flex items-center gap-10">
          <PanelThumbnail id={item.id} hasFile={file !== null} />
          <TitleAndDetail title={item.title} detail={item.detail} />
        </span>
      </RowButton>
      {file !== null && (
        <HoverHint text="Show in File Manager" className="mr-10 flex">
          <button
            type="button"
            aria-label={`Show ${item.title} in File Manager`}
            onClick={reveal}
            className={cx("flex text-muted enabled:cursor-pointer hovered:text-text", focusRing)}
          >
            <Folder aria-hidden size={13} />
          </button>
        </HoverHint>
      )}
      {menuAt && (
        <ContextMenu
          x={menuAt.x}
          y={menuAt.y}
          items={menu}
          onClose={() => {
            setMenuAt(null);
          }}
        />
      )}
    </div>
  );
}
