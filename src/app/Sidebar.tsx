import { useQueryClient } from "@tanstack/react-query";
import { PanelLeft } from "lucide-react";
import { useState, type KeyboardEvent, type PointerEvent, type RefObject } from "react";

import { commands, type SidebarLayout } from "../bindings";
import { BrandMark, SidebarRow } from "../design/Components";
import { cx } from "../design/cx";
import { HoverHint } from "../design/HoverHint";
import { useCommand } from "../lib/ipc";
import { queryKeys } from "../lib/queryKeys";
import { AccountPlate } from "./AccountPlate";
import { useAccountPlate, useLiveJobs } from "./liveData";
import { pages, screens, type Screen, type ScreenId } from "./screens";

const settings = screens.find((screen) => screen.id === "settings");

/** The sidebar's layout and its changes: shown or hidden, and the width it is dragged to. */
export interface SidebarState {
  /** `undefined` until Rust answered; the sidebar shows at its narrowest meanwhile. */
  layout: SidebarLayout | undefined;
  /** The width while it is dragged. */
  dragWidth: number | null;
  toggle: () => void;
  drag: (width: number) => void;
  /** Keeps the width the drag ended at. */
  endDrag: () => void;
}

/** `@SceneStorage("sidebarCollapsed")` and the split view's width, kept by Rust across launches. */
export function useSidebarLayout(): SidebarState {
  const client = useQueryClient();
  const layout = useCommand(queryKeys.settings.sidebar, () => commands.sidebarLayout()).data ?? undefined;
  const [dragWidth, setDragWidth] = useState<number | null>(null);
  const store = (isCollapsed: boolean | null, width: number | null) => {
    commands.setSidebarLayout(isCollapsed, width).then(
      (next) => {
        client.setQueryData(queryKeys.settings.sidebar, next);
        setDragWidth(null);
      },
      () => {
        setDragWidth(null);
      },
    );
  };
  return {
    layout,
    dragWidth,
    toggle: () => {
      const isCollapsed = layout?.isCollapsed ?? false;
      if (layout) client.setQueryData(queryKeys.settings.sidebar, { ...layout, isCollapsed: !isCollapsed });
      store(!isCollapsed, null);
    },
    drag: (width) => {
      if (!layout) return;
      setDragWidth(Math.min(Math.max(width, layout.minWidth), layout.maxWidth));
    },
    endDrag: () => {
      if (dragWidth !== null) store(null, dragWidth);
    },
  };
}

/**
 * The sidebar of RootView.swift: the brand mark, the pages with the Queue's badge, the account plate and Settings.
 * One tab stop; Up and Down step through the screens and stop at the ends. The focus ring shows once the arrow keys
 * are used, not after a click. Its toggle hides it (View › Toggle Sidebar, Ctrl+Shift+S), and its edge drags it wider.
 */
export function Sidebar({
  selection,
  onSelect,
  navRef,
  layout,
}: {
  selection: ScreenId | undefined;
  onSelect: (screen: Screen) => void;
  navRef: RefObject<HTMLElement | null>;
  layout: SidebarState;
}) {
  const plate = useAccountPlate();
  const { activeCount } = useLiveJobs();
  const [isFocused, setIsFocused] = useState(false);
  const [isKeyboardNavigating, setIsKeyboardNavigating] = useState(false);
  const isCollapsed = layout.layout?.isCollapsed ?? false;
  const width = layout.dragWidth ?? layout.layout?.width;

  const move = (event: KeyboardEvent<HTMLElement>) => {
    const delta = event.key === "ArrowUp" ? -1 : event.key === "ArrowDown" ? 1 : 0;
    if (delta === 0) return;
    event.preventDefault();
    setIsKeyboardNavigating(true);
    const current = Math.max(
      screens.findIndex((screen) => screen.id === selection),
      0,
    );
    const next = screens[Math.min(Math.max(current + delta, 0), screens.length - 1)];
    if (next) onSelect(next);
  };

  const row = (screen: Screen) => (
    <SidebarRow
      title={screen.title}
      icon={screen.icon}
      isSelected={selection === screen.id}
      isFocused={isFocused && isKeyboardNavigating && selection === screen.id}
      badge={screen.id === "queue" ? activeCount : 0}
      onClick={() => {
        onSelect(screen);
        setIsKeyboardNavigating(false);
        navRef.current?.focus();
      }}
    />
  );

  const toggle = layout.layout && (
    <SidebarToggle
      hint={layout.layout.toggleHint}
      label={layout.layout.toggleLabel}
      onToggle={layout.toggle}
      className={isCollapsed ? "absolute top-2 left-2 z-10" : undefined}
    />
  );

  if (isCollapsed) return toggle ?? null;

  return (
    <nav
      ref={navRef}
      tabIndex={0}
      onKeyDown={move}
      onFocus={() => {
        setIsFocused(true);
      }}
      onBlur={() => {
        setIsFocused(false);
      }}
      style={width === undefined ? undefined : { width }}
      className="relative flex w-(--sidebar-width) shrink-0 flex-col gap-2 border-r border-stroke bg-background px-10 py-14 outline-none"
    >
      <div className="flex items-start justify-between gap-small px-6 pb-18">
        <BrandMark />
        {toggle}
      </div>
      {pages.map((screen) => (
        <div key={screen.id}>{row(screen)}</div>
      ))}
      <div className="flex-1" />
      {plate && (
        <div className="pb-small">
          <AccountPlate
            plate={plate}
            onOpen={() => {
              if (settings) onSelect(settings);
            }}
          />
        </div>
      )}
      {settings && (
        <HoverHint text="Settings (Ctrl+,)" className="flex w-full">
          {row(settings)}
        </HoverHint>
      )}
      <ResizeEdge layout={layout} />
    </nav>
  );
}

/** The button that shows and hides the sidebar (`SidebarToggle`). */
function SidebarToggle({
  hint,
  label,
  onToggle,
  className,
}: {
  hint: string;
  label: string;
  onToggle: () => void;
  className?: string | undefined;
}) {
  return (
    <HoverHint text={hint} className={cx("flex", className)}>
      <button
        type="button"
        aria-label={label}
        onClick={(event) => {
          event.stopPropagation();
          onToggle();
        }}
        className="flex h-22 w-26 items-center justify-center rounded-hint text-muted hover:bg-well hover:text-text"
      >
        <PanelLeft size={14} strokeWidth={2.2} aria-hidden />
      </button>
    </HoverHint>
  );
}

/** The sidebar's right edge, dragged to make it wider or narrower within Rust's bounds. */
function ResizeEdge({ layout }: { layout: SidebarState }) {
  const [start, setStart] = useState<{ x: number; width: number } | null>(null);
  const width = layout.dragWidth ?? layout.layout?.width;
  if (!layout.layout || width === undefined) return null;
  const { minWidth, maxWidth } = layout.layout;
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="Sidebar width"
      aria-valuemin={minWidth}
      aria-valuemax={maxWidth}
      aria-valuenow={width}
      onPointerDown={(event: PointerEvent<HTMLDivElement>) => {
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
        setStart({ x: event.clientX, width });
      }}
      onPointerMove={(event: PointerEvent<HTMLDivElement>) => {
        if (start) layout.drag(start.width + event.clientX - start.x);
      }}
      onPointerUp={() => {
        setStart(null);
        layout.endDrag();
      }}
      className="absolute inset-y-0 -right-3 w-6 cursor-col-resize"
    />
  );
}
