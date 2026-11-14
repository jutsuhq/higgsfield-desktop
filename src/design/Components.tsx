import type { LucideIcon } from "lucide-react";
import { useState, type ButtonHTMLAttributes, type HTMLAttributes, type ReactNode } from "react";

import appIcon from "./assets/app-icon.png";
import { cx } from "./cx";
import { HoverHint } from "./HoverHint";
import { useReducedMotion } from "./motion";

/** `JobRowModel.ColorRole`: the colors a status may take. */
export type ColorRole = "accent" | "info" | "warning" | "danger" | "faint";

const dotClass: Record<ColorRole, string> = {
  accent: "bg-accent",
  info: "bg-info",
  warning: "bg-warning",
  danger: "bg-danger",
  faint: "bg-faint",
};

/** The big uppercase title and a muted line under it (`.head`). Trailing controls move under the title when the
 * page is too narrow for both. */
export function PageHeader({
  title,
  subtitle,
  trailing,
}: {
  title: string;
  subtitle?: string | undefined;
  trailing?: ReactNode;
}) {
  return (
    <header className="flex flex-wrap items-end gap-x-large gap-y-medium pb-x-large">
      <div className="mr-auto flex min-w-0 flex-col gap-x-small">
        <h1 className="m-0 font-display text-title font-heavy text-text uppercase">{title}</h1>
        {/* One line on every screen so the content below starts at the same height everywhere. */}
        <p className="m-0 truncate text-muted">{subtitle ?? "\u00a0"}</p>
      </div>
      {trailing}
    </header>
  );
}

/** A small uppercase caption above a group, with an optional count or price on the right (`.label`). */
export function SectionLabel({ title, detail }: { title: string; detail?: string | undefined }) {
  return (
    <div className="flex items-baseline gap-small text-faint">
      <h2 className="m-0 mr-auto text-label font-bold tracking-label uppercase">{title}</h2>
      {detail !== undefined && <span className="text-caption font-semibold tabular-nums">{detail}</span>}
    </div>
  );
}

/** A status or info capsule with an optional colored dot (`.pill`). The dot pulses while `isPulsing`. */
export function Pill({
  text,
  dot,
  isPulsing = false,
}: {
  text: string;
  dot?: ColorRole | undefined;
  isPulsing?: boolean;
}) {
  const reduceMotion = useReducedMotion();
  return (
    <span className="inline-flex h-24 shrink-0 items-center gap-6 rounded-full border-[0.5px] border-glass-stroke bg-glass px-10 text-caption font-semibold whitespace-nowrap text-text tabular-nums">
      {dot && (
        <span
          aria-hidden
          className={cx(
            "size-6 rounded-full",
            dotClass[dot],
            isPulsing && !reduceMotion && "animate-pulse-dot",
          )}
        />
      )}
      {text}
    </span>
  );
}

/** The price inside a primary button (`.cta .price`); inherits the button's ink. */
export function PriceTag({ text }: { text: string }) {
  return (
    <span className="rounded-full bg-ink-wash px-8 py-3 text-caption font-semibold tabular-nums">{text}</span>
  );
}

/** A tag drawn on a video frame, e.g. the mode, "Draft" or the duration (`.tag`). */
export function MediaTag({ text }: { text: string }) {
  return (
    <span className="inline-flex h-17 items-center rounded-full bg-scrim px-6 text-micro font-heavy tracking-tag whitespace-nowrap text-text uppercase">
      {text}
    </span>
  );
}

interface SidebarRowProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "title"> {
  title: string;
  icon: LucideIcon;
  isSelected: boolean;
  /** Draws the lime keyboard-focus ring (keyboard navigation in the sidebar, this row selected). */
  isFocused?: boolean;
  /** Active jobs; read as "Queue, 3 active". */
  badge?: number;
}

/** One sidebar destination (`.nav button`): muted until active, then a light fill and a lime icon. */
export function SidebarRow({
  title,
  icon: Icon,
  isSelected,
  isFocused = false,
  badge = 0,
  className,
  ...rest
}: SidebarRowProps) {
  return (
    <button
      type="button"
      tabIndex={-1}
      aria-label={badge > 0 ? `${title}, ${String(badge)} active` : title}
      aria-current={isSelected ? "page" : undefined}
      className={cx(
        "group relative flex h-32 w-full cursor-pointer items-center gap-10 rounded-row px-10 text-left text-body font-medium outline-none",
        isSelected ? "bg-selection text-text" : "text-muted hovered:bg-card hovered:text-text",
        className,
      )}
      {...rest}
    >
      <span className={cx("flex w-18 justify-center", isSelected ? "text-accent" : "text-muted")}>
        <Icon aria-hidden size={13} strokeWidth={2} />
      </span>
      <span className="mr-auto truncate">{title}</span>
      {badge > 0 && (
        <span
          aria-hidden
          className="inline-flex min-h-18 min-w-18 items-center justify-center rounded-full bg-accent px-5 text-label font-heavy text-ink tabular-nums"
        >
          {badge}
        </span>
      )}
      {isFocused && (
        <span
          aria-hidden
          data-testid="focus-ring"
          className="pointer-events-none absolute inset-2 rounded-row border-2 border-accent"
        />
      )}
    </button>
  );
}

/** The app icon and name at the top of the sidebar (`.brand`). The icon carries the signature gradient. */
export function BrandMark() {
  return (
    <div className="flex items-center gap-10">
      <img src={appIcon} alt="" aria-hidden className="size-28 rounded-app-icon" />
      <span className="font-display text-brand font-heavy tracking-brand whitespace-nowrap text-text uppercase">
        Jutsu Studio
      </span>
    </div>
  );
}

/** Soft signature light falling from the top of a page, for brand moments such as onboarding. */
export function SignatureBackdrop() {
  return <div aria-hidden className="pointer-events-none absolute inset-0 bg-(image:--signature-backdrop)" />;
}

/** The app icon large (96), floating on a signature glow (onboarding hero). */
export function GlowingAppIcon() {
  return (
    <div aria-hidden className="relative size-96 rounded-app-icon">
      <div className="absolute -inset-[70%] bg-(image:--signature-halo)" />
      <img src={appIcon} alt="" className="relative size-96 rounded-app-icon" />
    </div>
  );
}

/** "42%" for 0.42, rounded half up (`CostText.percent(fraction:)`). */
export function percentText(fraction: number): string {
  return `${String(Math.round(fraction * 100))}%`;
}

/**
 * Progress of a job while it renders: the signature gradient on a thin track (`.track`). With an `estimate` it is
 * filled against a typical time rather than reported progress: drawn faded lime, and read out as the estimate.
 */
export function RenderProgress({ value, estimate }: { value: number; estimate?: string | undefined }) {
  const fraction = Math.min(Math.max(value, 0), 1);
  return (
    <div
      role="progressbar"
      aria-label={estimate === undefined ? "Progress" : "Estimated time"}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(fraction * 100)}
      aria-valuetext={estimate ?? percentText(fraction)}
      className="h-4 w-full overflow-hidden rounded-full bg-track"
    >
      <div
        className={cx(
          "h-full rounded-full",
          estimate === undefined ? "bg-(image:--signature)" : "bg-accent-estimate",
        )}
        style={{ width: `max(var(--spacing-x-small), ${String(fraction * 100)}%)` }}
      />
    </div>
  );
}

/** `JobRowModel.TimeEstimate`. */
export interface TimeEstimate {
  /** "usually ~2 min", or "taking longer than usual" once past it. */
  label: string;
  /** Time since submit over the typical time, 0…1. */
  fraction: number;
  isOverdue: boolean;
  /** Where the estimate comes from, for the tooltip. */
  help: string;
}

/** A generating job against the typical time of similar past jobs, labelled as an estimate. */
export function EstimatedProgress({ estimate }: { estimate: TimeEstimate }) {
  return (
    <HoverHint text={estimate.help} className="flex w-full">
      <div className="flex w-full flex-col gap-x-small">
        <RenderProgress value={estimate.fraction} estimate={estimate.label} />
        <span
          aria-hidden
          className={cx(
            "truncate text-caption-plus font-medium",
            estimate.isOverdue ? "text-warning" : "text-faint",
          )}
        >
          {`${estimate.label} · estimate`}
        </span>
      </div>
    </HoverHint>
  );
}

const glowRadius = { card: "rounded-card", media: "rounded-media", field: "rounded-field" } as const;

/** A brief signature glow behind a result that has just finished. Starts lit and fades once on appear; with
 * Reduce Motion it is gone at once. */
export function FreshGlow({
  isFresh,
  radius = "card",
  children,
  className,
}: {
  isFresh: boolean;
  radius?: keyof typeof glowRadius;
  children: ReactNode;
  className?: string;
}) {
  const reduceMotion = useReducedMotion();
  // Decided once, on appear, like the Swift view's initial state.
  const [glows] = useState(isFresh);
  return (
    <div className={cx("relative", className)}>
      {glows && !reduceMotion && (
        <div
          aria-hidden
          className={cx(
            "pointer-events-none absolute inset-0 animate-fresh-glow bg-(image:--signature) opacity-0 blur-(--glow-blur)",
            glowRadius[radius],
          )}
        />
      )}
      <div className="relative">{children}</div>
    </div>
  );
}

/** An empty section: an icon tile, a line of text and an optional action (`.empty`). */
export function EmptyState({
  icon: Icon,
  title,
  detail,
  action,
  className,
}: {
  icon: LucideIcon;
  title: string;
  detail?: string | undefined;
  action?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cx("flex w-full flex-col items-center gap-small px-large py-28 text-center", className)}>
      <div className="mb-x-small flex size-44 items-center justify-center rounded-field bg-glass text-muted">
        <Icon aria-hidden size={18} strokeWidth={2} />
      </div>
      <p className="m-0 text-button font-semibold text-text">{title}</p>
      {detail !== undefined && <p className="m-0 text-control text-muted">{detail}</p>}
      {action !== undefined && <div className="pt-small">{action}</div>}
    </div>
  );
}

/** A dashed inset area that turns lime on hover or while files are dragged over it (`.drop`). */
export function DropZone({
  isHighlighted = false,
  className,
  children,
  ...rest
}: HTMLAttributes<HTMLDivElement> & { isHighlighted?: boolean }) {
  return (
    <div
      data-lit={isHighlighted || undefined}
      className={cx(
        "group relative rounded-field transition-colors duration-150 ease-out",
        "bg-well hovered:bg-accent-wash data-lit:bg-accent-wash",
        className,
      )}
      {...rest}
    >
      <svg aria-hidden className="pointer-events-none absolute inset-0 size-full overflow-visible">
        <rect
          className="fill-none stroke-glass-stroke transition-colors duration-150 ease-out group-hovered:stroke-accent-drop group-data-lit:stroke-accent-drop"
          style={{
            x: 0.6,
            y: 0.6,
            width: "calc(100% - 1.2px)",
            height: "calc(100% - 1.2px)",
            rx: "var(--radius-field)",
            strokeWidth: 1.2,
            strokeDasharray: "5 4",
          }}
        />
      </svg>
      {children}
    </div>
  );
}
