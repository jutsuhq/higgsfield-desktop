import type { ReactNode } from "react";

import { SectionLabel } from "../../design/Components";
import { cx } from "../../design/cx";
import { HoverHint } from "../../design/HoverHint";
import { focusRing } from "../../design/Styles";

/** A titled card of rows with an optional note below (SettingsRows.swift `SettingsSection`, `.set`). */
export function SettingsSection({
  title,
  footer,
  children,
}: {
  title?: string;
  footer?: string;
  children: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-small">
      {title !== undefined && <SectionLabel title={title} />}
      <div className="flex flex-col rounded-[14px] border border-stroke bg-card px-large">{children}</div>
      {footer !== undefined && <p className="m-0 px-x-small text-[12px] text-faint">{footer}</p>}
    </section>
  );
}

/**
 * A label on the left and its control or value on the right, with a hairline above unless it is first. A disabled row
 * is dimmed (`.disabled(…).opacity(0.45)`); its control is disabled on its own.
 */
export function SettingsRow({
  label,
  detail,
  isFirst = false,
  isDisabled = false,
  children,
}: {
  label: string;
  detail?: string | undefined;
  isFirst?: boolean;
  isDisabled?: boolean;
  children?: ReactNode;
}) {
  return (
    <div
      className={cx(
        "flex min-h-50 items-center gap-large",
        !isFirst && "border-t border-stroke",
        isDisabled && "opacity-45",
      )}
    >
      <div className="flex min-w-0 flex-col gap-2">
        <span className="text-body font-medium text-text">{label}</span>
        {detail !== undefined && (
          <HoverHint text={detail} className="flex min-w-0">
            <span className="truncate text-caption-plus text-muted">{detail}</span>
          </HoverHint>
        )}
      </div>
      <span className="flex-1" />
      {children}
    </div>
  );
}

/** A read-only value row; key IDs read better in a fixed-width font. */
export function SettingsValueRow({
  label,
  value,
  isFirst = false,
  isMonospaced = false,
}: {
  label: string;
  value: string;
  isFirst?: boolean;
  isMonospaced?: boolean;
}) {
  return (
    <SettingsRow label={label} isFirst={isFirst}>
      <span className={cx("text-body text-muted select-text", isMonospaced && "font-mono")}>{value}</span>
    </SettingsRow>
  );
}

/** `Toggle(...).toggleStyle(.switch)`: lime while on. */
export function Switch({
  label,
  isOn,
  onChange,
  disabled = false,
}: {
  label: string;
  isOn: boolean;
  onChange: (isOn: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={isOn}
      aria-label={label}
      disabled={disabled}
      onClick={() => {
        onChange(!isOn);
      }}
      className={cx(
        "relative h-22 w-38 shrink-0 rounded-full enabled:cursor-pointer border border-stroke transition-colors duration-150",
        isOn ? "bg-accent" : "bg-track",
        focusRing,
      )}
    >
      <span
        aria-hidden
        className={cx(
          "absolute top-1/2 size-18 -translate-y-1/2 rounded-full bg-text shadow-sm transition-[left] duration-150",
          isOn ? "left-17" : "left-1",
        )}
      />
    </button>
  );
}
