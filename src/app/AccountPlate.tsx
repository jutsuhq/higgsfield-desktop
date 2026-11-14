import {
  ChevronRight,
  CircleDashed,
  CircleUser,
  Crown,
  KeyRound,
  Sparkles,
  Star,
  TriangleAlert,
  Zap,
  type LucideIcon,
} from "lucide-react";
import type { CSSProperties } from "react";

import { cx } from "../design/cx";
import { HoverHint } from "../design/HoverHint";
import { useReducedMotion } from "../design/motion";
import { focusRing } from "../design/Styles";
import type { AccountPlateModel, PlanTier } from "./liveData";

/** How a tier dresses its badge and the plate (AccountPlateView.swift). */
interface TierLook {
  /** Left to right across the badge. */
  colors: string[];
  /** The plate's edge and corner light. */
  edge: string;
  icon: LucideIcon;
  /** The plate's colored shadow. */
  shadow?: string;
  shines: boolean;
}

const color = (name: string) => `var(--color-${name})`;

const looks: Record<PlanTier | "none", TierLook> = {
  none: {
    colors: [color("tier-free-from"), color("tier-free-to")],
    edge: color("tier-free-edge"),
    icon: CircleUser,
    shines: false,
  },
  free: {
    colors: [color("tier-free-from"), color("tier-free-to")],
    edge: color("tier-free-edge"),
    icon: CircleDashed,
    shines: false,
  },
  starter: {
    colors: [color("tier-starter-from"), color("tier-starter-to")],
    edge: color("tier-starter-to"),
    icon: Zap,
    shines: false,
  },
  pro: {
    colors: [color("tier-pro-from"), color("tier-pro-to")],
    edge: color("tier-pro-to"),
    icon: Star,
    shines: false,
  },
  premium: {
    colors: [color("tier-premium-from"), color("tier-premium-mid"), color("tier-premium-to")],
    edge: color("tier-premium-mid"),
    icon: Crown,
    shadow: "shadow-tier-premium",
    shines: true,
  },
  elite: {
    colors: [color("signature-violet"), color("signature-fuchsia"), color("signature-pink")],
    edge: color("signature-fuchsia"),
    icon: Sparkles,
    shadow: "shadow-tier-elite",
    shines: true,
  },
};

const fade = (value: string, percent: number) =>
  `color-mix(in srgb, ${value} ${String(percent)}%, transparent)`;

/** A 1 px border painted with `background`, drawn inside the element's rounded box. */
const ringMask: CSSProperties = {
  padding: 1,
  mask: "linear-gradient(var(--color-text) 0 0) content-box exclude, linear-gradient(var(--color-text) 0 0)",
};

/** The plan and the credits left, above Settings in the sidebar. Opens Settings, where the connection is. */
export function AccountPlate({
  plate,
  onOpen,
  ...rest
}: {
  plate: AccountPlateModel;
  onOpen: () => void;
  "data-hover"?: boolean;
}) {
  const look = looks[plate.tier ?? "none"];
  return (
    <HoverHint
      text={plate.warning === null ? "Open Settings" : "Top up on higgsfield.ai"}
      className="flex w-full"
    >
      <button
        type="button"
        aria-label={plate.accessibilityLabel}
        aria-description="Opens Settings"
        onClick={onOpen}
        data-hover={rest["data-hover"]}
        className={cx(
          "group relative flex w-full cursor-pointer flex-col gap-10 overflow-hidden rounded-media p-medium text-left",
          "bg-card transition-colors duration-120 ease-out hovered:bg-glass",
          look.shadow,
          focusRing,
        )}
      >
        <span
          aria-hidden
          className="pointer-events-none absolute inset-0"
          style={{
            background: `radial-gradient(circle var(--plate-light-radius) at 0 0, ${fade(look.edge, 24)}, transparent)`,
          }}
        />
        <span
          aria-hidden
          className="pointer-events-none absolute inset-0 rounded-media opacity-100 group-hovered:opacity-0"
          style={{
            ...ringMask,
            background: `linear-gradient(to bottom right, ${fade(look.edge, 50)}, ${fade(look.edge, 6)})`,
          }}
        />
        <span
          aria-hidden
          className="pointer-events-none absolute inset-0 rounded-media opacity-0 group-hovered:opacity-100"
          style={{
            ...ringMask,
            background: `linear-gradient(to bottom right, ${fade(look.edge, 70)}, ${fade(look.edge, 6)})`,
          }}
        />
        <span className="relative flex w-full items-center gap-6">
          <span className="mr-auto">
            <PlanBadge title={plate.badge} tier={plate.tier} icon={plate.isAPIKey ? KeyRound : undefined} />
          </span>
          <ChevronRight
            aria-hidden
            size={9}
            strokeWidth={3.5}
            className="text-faint group-hovered:text-muted"
          />
        </span>
        <span className="relative flex flex-col gap-2">
          <span className="flex items-baseline gap-5">
            <span
              className={cx(
                "truncate font-display font-heavy tabular-nums",
                plate.isAPIKey ? "text-plate-key" : "text-plate",
                plate.warning === null ? "text-text" : "text-warning",
              )}
            >
              {plate.amount}
            </span>
            <span className="text-caption-plus font-semibold whitespace-nowrap text-faint">{plate.unit}</span>
          </span>
          {plate.warning !== null && (
            <span className="flex items-center gap-x-small text-caption font-medium text-warning tabular-nums">
              <TriangleAlert
                aria-hidden
                size={11}
                strokeWidth={2.5}
                className="shrink-0 fill-warning text-background"
              />
              {plate.warning}
            </span>
          )}
        </span>
      </button>
    </HoverHint>
  );
}

/**
 * A plan's name in a capsule dressed for its tier: plain for free, colored for paid plans, metal for the top ones,
 * and the signature gradient with a passing shine for the highest.
 */
export function PlanBadge({
  title,
  tier,
  icon,
}: {
  title: string;
  tier: PlanTier | null;
  icon?: LucideIcon | undefined;
}) {
  const reduceMotion = useReducedMotion();
  const look = looks[tier ?? "none"];
  const Icon = icon ?? look.icon;
  const gradient = `linear-gradient(to right, ${look.colors.join(", ")})`;
  const isElite = tier === "elite";
  const isPlain = tier === null || tier === "free";
  return (
    <span className="relative inline-flex h-22 items-center gap-5 overflow-hidden rounded-full px-9 whitespace-nowrap">
      <span
        aria-hidden
        className={cx("absolute inset-0", !isElite && (isPlain ? "opacity-10" : "opacity-16"))}
        style={{ background: gradient }}
      />
      <span
        aria-hidden
        className={cx("absolute inset-0 rounded-full", !isElite && "opacity-55")}
        style={{
          ...ringMask,
          padding: 0.8,
          background: isElite ? "var(--color-tier-elite-stroke)" : gradient,
        }}
      />
      <Icon
        aria-hidden
        size={9}
        strokeWidth={3}
        fill={icon === undefined && tier !== null && tier !== "free" ? "currentColor" : "none"}
        className="relative"
        style={{ color: isElite ? "var(--color-text)" : look.colors[0] }}
      />
      <span
        className="relative text-tiny font-heavy tracking-label uppercase"
        style={
          isElite
            ? { color: "var(--color-text)" }
            : { backgroundImage: gradient, backgroundClip: "text", color: "transparent" }
        }
      >
        {title}
      </span>
      {look.shines && !reduceMotion && (
        <span
          aria-hidden
          className="pointer-events-none absolute inset-y-[-50%] left-0 w-[45%] animate-plan-shine mix-blend-plus-lighter"
          style={{
            background: `linear-gradient(to right, transparent, var(--color-${isElite ? "shine-elite" : "shine"}), transparent)`,
          }}
        />
      )}
    </span>
  );
}
