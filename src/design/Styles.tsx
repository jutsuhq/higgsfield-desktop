import { Check, Copy, type LucideIcon } from "lucide-react";
import {
  useEffect,
  useState,
  type ButtonHTMLAttributes,
  type HTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
} from "react";

import { PriceTag } from "./Components";
import { cx } from "./cx";
import { HoverHint } from "./HoverHint";

/**
 * Styles.swift's button styles:
 * - `primary`: the lime call to action (`.cta.lime`), one per screen. Lime with a glow; brightens on hover. Disabled
 *   it drops to a flat glass capsule with faint text.
 * - `secondary`: everything else clickable (`.btn.glass`). `destructive` turns the label red.
 * - `icon`: a round glass button holding one symbol; the title is its accessible name.
 * - `textLink`: an inline text action in lime.
 */
export type ButtonVariant = "primary" | "secondary" | "icon" | "textLink";

export interface ButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> {
  variant: ButtonVariant;
  title: string;
  icon?: LucideIcon | undefined;
  /** The price chip at the end of a primary button. */
  price?: string | undefined;
  /** Plain muted text after the title, in tabular digits: the price on a secondary button (`GenerateButtonLabel`). */
  detail?: string | undefined;
  /** `role: .destructive`. */
  destructive?: boolean;
  /** The app's tooltip (`hoverHint`). */
  hint?: string | undefined;
  /** For SF Symbols drawn filled, e.g. `pause.fill`. */
  isIconFilled?: boolean;
  /** Drawn before the icon, e.g. a spinner while the action runs. */
  leading?: ReactNode;
  /** Shows the icon alone in the variant's own shape (`Label(…).labelStyle(.iconOnly)` or an `Image` label), the title
   *  read out instead. The `icon` variant is always icon-only. */
  isLabelHidden?: boolean;
}

/** The keyboard focus ring every control shares: lime, 2 px, drawn inside the control as `focusRing` in Styles.swift. */
export const focusRing =
  "outline-hidden focused:outline-2 focused:-outline-offset-4 focused:outline-accent focused:outline-solid";

const variantClass: Record<ButtonVariant, string> = {
  primary: cx(
    "min-h-42 gap-6 rounded-full px-18 text-button font-heavy",
    "enabled:bg-accent enabled:text-ink enabled:shadow-accent-glow",
    "hovered:shadow-accent-glow-hover hovered:bg-accent-hover pressed:bg-accent",
    "disabled:border disabled:border-stroke disabled:bg-glass disabled:text-faint",
    "motion-safe:pressed:scale-98",
  ),
  secondary: cx(
    "min-h-34 gap-6 rounded-full border border-stroke px-14 text-body font-semibold",
    "bg-glass hovered:border-glass-stroke hovered:bg-glass-hover pressed:bg-glass-pressed",
    "disabled:bg-well disabled:opacity-40 motion-safe:pressed:scale-98",
  ),
  icon: cx(
    "size-34 shrink-0 rounded-full border border-stroke text-body font-semibold",
    "bg-glass hovered:border-glass-stroke hovered:bg-glass-hover pressed:bg-glass-pressed",
    "disabled:bg-well disabled:opacity-40 motion-safe:pressed:scale-96",
  ),
  textLink:
    "text-small font-medium text-accent opacity-90 hovered:underline hovered:opacity-100 pressed:opacity-70",
};

const iconSize: Record<ButtonVariant, number> = { primary: 14, secondary: 13, icon: 13, textLink: 12 };

export function Button({
  variant,
  title,
  icon: Icon,
  price,
  detail,
  destructive = false,
  hint,
  isIconFilled = false,
  leading,
  isLabelHidden = false,
  className,
  type = "button",
  ...rest
}: ButtonProps) {
  const isIconOnly = variant === "icon" || isLabelHidden;
  const color =
    variant === "secondary" || variant === "icon" ? (destructive ? "text-danger" : "text-text") : undefined;
  const button = (
    <button
      type={type}
      aria-label={isIconOnly ? title : undefined}
      className={cx(
        "inline-flex items-center justify-center whitespace-nowrap select-none enabled:cursor-pointer",
        "transition-[background-color,border-color,box-shadow,filter,opacity,scale] duration-120 ease-out",
        focusRing,
        variantClass[variant],
        color,
        className,
      )}
      {...rest}
    >
      {leading}
      {Icon && (
        <Icon
          aria-hidden
          size={iconSize[variant]}
          strokeWidth={variant === "primary" ? 2.75 : 2.25}
          fill={isIconFilled ? "currentColor" : "none"}
        />
      )}
      {!isIconOnly && <span className="truncate">{title}</span>}
      {detail !== undefined && <span className="text-muted tabular-nums">{detail}</span>}
      {price !== undefined && <PriceTag text={price} />}
    </button>
  );
  return hint === undefined ? button : <HoverHint text={hint}>{button}</HoverHint>;
}

type CardRadius = "card" | "media" | "field";

const radiusClass: Record<CardRadius, string> = {
  card: "rounded-card",
  media: "rounded-media",
  field: "rounded-field",
};

interface SurfaceProps extends HTMLAttributes<HTMLDivElement> {
  radius?: CardRadius;
  children?: ReactNode;
}

/** A raised surface (`.glass.dim`): job rows, onboarding cards. A highlighted card has a 2 px lime edge. */
export function Card({
  radius = "card",
  isHighlighted = false,
  className,
  ...rest
}: SurfaceProps & { isHighlighted?: boolean }) {
  return (
    <div
      className={cx(
        "bg-card",
        radiusClass[radius],
        isHighlighted ? "border-2 border-accent" : "border border-stroke",
        className,
      )}
      {...rest}
    />
  );
}

/** An inset area (`--well`): empty states. */
export function Well({ radius = "field", className, ...rest }: SurfaceProps) {
  return <div className={cx("border border-stroke bg-well", radiusClass[radius], className)} {...rest} />;
}

/** Page padding: `page` on the sides and bottom, `pageTop` above the header. */
export const pagePadding = "px-page pt-page-top pb-page";

/**
 * `themedField`: a text field drawn as a well with a lime border while focused, instead of the system rounded border
 * and its focus ring.
 */
export function TextField({ className, ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      className={cx(
        "h-38 rounded-field border border-stroke bg-well px-medium text-body text-text outline-none",
        // As wide as its parent unless the caller sets a width: two width classes would leave the winner to chance.
        !/(^|\s)w-/u.test(className ?? "") && "w-full",
        "transition-[border-color,box-shadow] duration-150 ease-out placeholder:text-faint",
        // A 1.5 px lime edge, drawn inside so the text doesn't move.
        "focus:border-accent focus:shadow-[inset_0_0_0_0.5px_var(--color-accent)]",
        "data-focus:border-accent data-focus:shadow-[inset_0_0_0_0.5px_var(--color-accent)]",
        "disabled:opacity-40",
        className,
      )}
      {...rest}
    />
  );
}

/** Copy, then Copied with a check for a moment (`CopyButton` in ResultSummary.swift). */
export function CopyButton({ text }: { text: string }) {
  const [didCopy, setDidCopy] = useState(false);
  useEffect(() => {
    if (!didCopy) return;
    const timer = setTimeout(() => {
      setDidCopy(false);
    }, 1500);
    return () => {
      clearTimeout(timer);
    };
  }, [didCopy]);
  const Icon = didCopy ? Check : Copy;
  return (
    <button
      type="button"
      className={cx(
        "flex cursor-pointer items-center gap-4 text-caption font-semibold",
        didCopy ? "text-accent" : "text-muted",
        focusRing,
      )}
      onClick={() => {
        void navigator.clipboard.writeText(text);
        setDidCopy(true);
      }}
    >
      <Icon aria-hidden size={11} strokeWidth={2.5} />
      {didCopy ? "Copied" : "Copy"}
    </button>
  );
}
