import { Minus, Plus, type LucideIcon } from "lucide-react";
import type { KeyboardEvent } from "react";

import { cx } from "./cx";
import { focusRing } from "./Styles";

interface ValueStepperProps {
  /** What the value is, e.g. "Variants"; the buttons read "Decrease Variants" and "Increase Variants". */
  label: string;
  value: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
}

/** Minus, the value and plus in one capsule, in place of the system stepper. */
export function ValueStepper({ label, value, min, max, onChange }: ValueStepperProps) {
  const adjust = (delta: number) => {
    const next = value + delta;
    if (next >= min && next <= max) onChange(next);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLSpanElement>) => {
    if (event.key === "ArrowUp" || event.key === "ArrowRight") {
      event.preventDefault();
      adjust(1);
    } else if (event.key === "ArrowDown" || event.key === "ArrowLeft") {
      event.preventDefault();
      adjust(-1);
    }
  };

  return (
    <div className="inline-flex items-center rounded-full border border-stroke bg-well p-3">
      <StepButton
        icon={Minus}
        title={`Decrease ${label}`}
        disabled={value - 1 < min}
        onClick={() => {
          adjust(-1);
        }}
      />
      <span
        role="spinbutton"
        tabIndex={0}
        aria-label={label}
        aria-valuenow={value}
        aria-valuemin={min}
        aria-valuemax={max}
        onKeyDown={onKeyDown}
        className={cx(
          "min-w-28 rounded-full text-center text-body font-semibold text-text tabular-nums",
          focusRing,
        )}
      >
        {value}
      </span>
      <StepButton
        icon={Plus}
        title={`Increase ${label}`}
        disabled={value + 1 > max}
        onClick={() => {
          adjust(1);
        }}
      />
    </div>
  );
}

function StepButton({
  icon: Icon,
  title,
  disabled,
  onClick,
}: {
  icon: LucideIcon;
  title: string;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={title}
      disabled={disabled}
      onClick={onClick}
      className={cx(
        "flex size-28 items-center justify-center rounded-full enabled:cursor-pointer",
        "text-text hovered:bg-glass pressed:bg-glass-pressed disabled:text-faint",
        focusRing,
      )}
    >
      <Icon aria-hidden size={10} strokeWidth={3.5} />
    </button>
  );
}
