import { Check, ChevronDown, CircleX, Search, type LucideIcon } from "lucide-react";
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type HTMLAttributes,
  type KeyboardEvent,
  type RefObject,
} from "react";

import { cx } from "./cx";
import { HoverHint } from "./HoverHint";
import { focusRing } from "./Styles";

export interface Option<Value> {
  value: Value;
  title: string;
}

interface SegmentedControlProps<Value> extends Omit<HTMLAttributes<HTMLDivElement>, "onChange"> {
  value: Value;
  options: readonly Option<Value>[];
  onChange: (value: Value) => void;
  /** `.fixedSize()`: as wide as the titles. Otherwise the segments share the available width. */
  isCompact?: boolean;
}

/**
 * A capsule track with a lime pill on the chosen option (`.seg`). The pill slides to a new choice (snappy, 0.22 s)
 * unless Reduce Motion is on. A segment is never narrower than its title.
 */
export function SegmentedControl<Value extends string | number>({
  value,
  options,
  onChange,
  isCompact = false,
  className,
  ...rest
}: SegmentedControlProps<Value>) {
  const track = useRef<HTMLDivElement>(null);
  const [pill, setPill] = useState<{ left: number; width: number; animates: boolean }>();
  const selectedIndex = options.findIndex((option) => option.value === value);

  const measure = useCallback(() => {
    const button = track.current?.children.item(selectedIndex + 1);
    if (!(button instanceof HTMLElement)) return;
    setPill((previous) => ({
      left: button.offsetLeft,
      width: button.offsetWidth,
      animates: previous !== undefined,
    }));
  }, [selectedIndex]);

  useLayoutEffect(measure, [measure]);
  useEffect(() => {
    if (typeof ResizeObserver === "undefined" || !track.current) return;
    const observer = new ResizeObserver(measure);
    observer.observe(track.current);
    return () => {
      observer.disconnect();
    };
  }, [measure]);

  return (
    <div
      ref={track}
      className={cx(
        "relative gap-2 rounded-full border border-stroke bg-well p-3",
        isCompact ? "inline-flex" : "flex w-full",
        className,
      )}
      {...rest}
    >
      <span
        aria-hidden
        className={cx(
          "absolute top-3 bottom-3 rounded-full bg-accent",
          pill?.animates && "transition-[left,width] duration-220 ease-snappy",
          (pill === undefined || pill.width === 0) && "hidden",
        )}
        style={pill && { left: pill.left, width: pill.width }}
      />
      {options.map((option) => {
        const isSelected = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            aria-pressed={isSelected}
            onClick={() => {
              onChange(option.value);
            }}
            className={cx(
              "relative min-h-28 cursor-pointer rounded-full px-10 text-control whitespace-nowrap",
              "transition-colors duration-120 ease-out",
              focusRing,
              isCompact ? "shrink-0" : "flex-1",
              isSelected
                ? "font-bold text-ink"
                : "font-semibold text-muted hovered:bg-glass hovered:text-text",
              // Before the pill is measured, and where it can't be (no layout), the segment draws its own.
              isSelected && (pill === undefined || pill.width === 0) && "bg-accent",
            )}
          >
            {option.title}
          </button>
        );
      })}
    </div>
  );
}

interface FilterMenuProps<Value> {
  /** The menu's name, read out with the current choice. */
  title: string;
  value: Value;
  /** The first option is the "any" choice: picking another turns the label lime. */
  options: readonly Option<Value>[];
  onChange: (value: Value) => void;
  /** Forces the hover look, for the design review page. */
  "data-hover"?: boolean;
}

/**
 * A pull-down filter: a capsule with the current choice and a chevron, opening a menu with checkmarks (`.filter`).
 * The label turns lime once a choice other than the first (the "any" option) is picked.
 */
export function FilterMenu<Value extends string | number | null>({
  title,
  value,
  options,
  onChange,
  ...rest
}: FilterMenuProps<Value>) {
  const [isOpen, setIsOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const valueId = useId();
  const isFiltering = options.length > 0 && value !== options[0]?.value;
  const choice = options.find((option) => option.value === value)?.title;

  useEffect(() => {
    if (!isOpen) return;
    const items = menu.current?.querySelectorAll<HTMLElement>("[role=menuitemradio]");
    const checked = menu.current?.querySelector<HTMLElement>("[aria-checked=true]");
    (checked ?? items?.item(0))?.focus();
    const close = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setIsOpen(false);
    };
    document.addEventListener("pointerdown", close);
    return () => {
      document.removeEventListener("pointerdown", close);
    };
  }, [isOpen]);

  const closeMenu = () => {
    setIsOpen(false);
    button.current?.focus();
  };

  const onMenuKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const items = [...(menu.current?.querySelectorAll<HTMLElement>("[role=menuitemradio]") ?? [])];
    const index = items.indexOf(document.activeElement as HTMLElement);
    if (event.key === "Escape" || event.key === "Tab") {
      event.preventDefault();
      closeMenu();
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      items[Math.min(index + 1, items.length - 1)]?.focus();
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      items[Math.max(index - 1, 0)]?.focus();
    }
  };

  return (
    <div ref={root} className="relative inline-flex shrink-0">
      <button
        ref={button}
        type="button"
        aria-label={title}
        aria-describedby={valueId}
        aria-haspopup="menu"
        aria-expanded={isOpen}
        data-hover={rest["data-hover"]}
        onClick={() => {
          setIsOpen((open) => !open);
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown") {
            event.preventDefault();
            setIsOpen(true);
          }
        }}
        className={cx(
          "inline-flex h-34 cursor-pointer items-center gap-6 rounded-full border px-14 whitespace-nowrap",
          "bg-well transition-colors duration-120 ease-out hovered:bg-glass-hover",
          focusRing,
          isFiltering ? "border-accent-edge text-accent" : "border-stroke text-muted hovered:text-text",
        )}
      >
        <span id={valueId} className="text-control font-semibold">
          {choice ?? title}
        </span>
        <ChevronDown
          aria-hidden
          size={9}
          strokeWidth={3.5}
          className={isFiltering ? "text-accent" : "text-faint"}
        />
      </button>
      {isOpen && (
        <div
          ref={menu}
          role="menu"
          aria-label={title}
          onKeyDown={onMenuKey}
          className="absolute top-full left-0 z-40 mt-x-small flex min-w-full flex-col rounded-field border border-glass-stroke bg-background p-x-small shadow-toast"
        >
          {options.map((option) => {
            const isChecked = option.value === value;
            return (
              <button
                key={String(option.value)}
                type="button"
                role="menuitemradio"
                aria-checked={isChecked}
                onClick={() => {
                  onChange(option.value);
                  closeMenu();
                }}
                className="flex h-24 cursor-pointer items-center gap-6 rounded-small px-small text-left text-body whitespace-nowrap text-text outline-none hover:bg-selection focus:bg-selection"
              >
                <Check aria-hidden size={12} strokeWidth={2.5} className={isChecked ? "" : "invisible"} />
                {option.title}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

interface FilterToggleProps extends Omit<HTMLAttributes<HTMLButtonElement>, "onChange"> {
  title: string;
  icon: LucideIcon;
  /** The glyph while on, where the Swift caller swaps the symbol (eye → eye.slash). Without it the icon is filled. */
  iconOn?: LucideIcon;
  isOn: boolean;
  onChange: (isOn: boolean) => void;
}

/** An on-or-off filter, as a capsule like `FilterMenu`'s that turns lime while on. */
export function FilterToggle({ title, icon, iconOn, isOn, onChange, className, ...rest }: FilterToggleProps) {
  const Icon = isOn && iconOn ? iconOn : icon;
  return (
    <button
      type="button"
      aria-pressed={isOn}
      onClick={() => {
        onChange(!isOn);
      }}
      className={cx(
        "inline-flex h-34 shrink-0 cursor-pointer items-center gap-6 rounded-full border px-14 whitespace-nowrap",
        "bg-well transition-colors duration-120 ease-out hovered:bg-glass-hover",
        focusRing,
        isOn ? "border-accent-edge text-accent" : "border-stroke text-muted hovered:text-text",
        className,
      )}
      {...rest}
    >
      <Icon aria-hidden size={10} strokeWidth={3} fill={isOn && !iconOn ? "currentColor" : "none"} />
      <span className="text-control font-semibold">{title}</span>
    </button>
  );
}

interface SearchFieldProps {
  value: string;
  onChange: (value: string) => void;
  /** The placeholder. */
  prompt: string;
  className?: string;
  /** Forces the focused look, for the design review page. */
  "data-focus"?: boolean;
  /** The input, for a page that focuses it through its own shortcut; Ctrl+F is then left to that page. */
  inputRef?: RefObject<HTMLInputElement | null>;
}

/**
 * A capsule search field with a magnifier and a clear button (`.search`). Ctrl+F focuses it; Escape clears it.
 */
export function SearchField({ value, onChange, prompt, className, inputRef, ...rest }: SearchFieldProps) {
  const ownInput = useRef<HTMLInputElement>(null);
  const input = inputRef ?? ownInput;
  const hasShortcut = inputRef === undefined;

  useEffect(() => {
    if (!hasShortcut) return;
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.ctrlKey && !event.altKey && !event.shiftKey && event.key.toLowerCase() === "f") {
        event.preventDefault();
        input.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [hasShortcut, input]);

  return (
    <div
      data-focus={rest["data-focus"]}
      className={cx(
        "group flex h-34 items-center gap-small rounded-full border border-stroke bg-well px-14",
        "transition-[border-color,box-shadow] duration-150 ease-out",
        "focus-within:border-accent focus-within:shadow-[inset_0_0_0_0.5px_var(--color-accent)]",
        "data-focus:border-accent data-focus:shadow-[inset_0_0_0_0.5px_var(--color-accent)]",
        className,
      )}
    >
      <Search
        aria-hidden
        size={12}
        strokeWidth={2.75}
        className="shrink-0 text-faint group-focus-within:text-accent group-data-focus:text-accent"
      />
      <input
        ref={input}
        type="text"
        value={value}
        placeholder={prompt}
        aria-label={prompt}
        onChange={(event) => {
          onChange(event.target.value);
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") onChange("");
        }}
        className="h-full min-w-0 flex-1 bg-transparent text-body text-text outline-none placeholder:text-faint"
      />
      {value !== "" && (
        <HoverHint text="Clear search">
          <button
            type="button"
            aria-label="Clear search"
            onClick={() => {
              onChange("");
              input.current?.focus();
            }}
            className={cx("flex cursor-pointer rounded-full", focusRing)}
          >
            <CircleX aria-hidden size={14} className="fill-faint stroke-background" />
          </button>
        </HoverHint>
      )}
    </div>
  );
}
