import { useEffect, useRef, type ReactNode } from "react";

import { commands, type WindowSheet } from "../bindings";
import { cx } from "../design/cx";

/**
 * The dialogs open now, outermost first. Rust is told the outermost one, the sheet over the window (`mainWindowHasSheet`),
 * so an agent's `show` doesn't open a sheet over it.
 */
const openSheets: { sheet: WindowSheet }[] = [];

function reportSheet() {
  void commands.agentsSetWindowSheet(openSheets[0]?.sheet ?? null);
}

/**
 * A sheet or alert over the window (`.sheet`, `.alert`, `.confirmationDialog`): dims the window, takes the focus and
 * closes with Esc through `onCancel`. `role` is `alertdialog` for alerts and confirmations.
 */
export function Dialog({
  title,
  role = "dialog",
  onCancel,
  className,
  sheet = "other",
  children,
}: {
  title: string;
  role?: "dialog" | "alertdialog";
  onCancel: () => void;
  className?: string;
  /** Which sheet this is, for an agent's `show`: one it may replace (`onDevice`, `subtitles`) or `other`. */
  sheet?: WindowSheet;
  children: ReactNode;
}) {
  const panel = useRef<HTMLDivElement>(null);
  const cancel = useRef(onCancel);
  useEffect(() => {
    cancel.current = onCancel;
  });

  useEffect(() => {
    const open = { sheet };
    openSheets.push(open);
    reportSheet();
    return () => {
      openSheets.splice(openSheets.indexOf(open), 1);
      reportSheet();
    };
  }, [sheet]);

  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const first = panel.current?.querySelector<HTMLElement>("[data-autofocus], button:not(:disabled)");
    (first ?? panel.current)?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      cancel.current();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      previous?.focus();
    };
  }, []);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-scrim px-page">
      <div
        ref={panel}
        role={role}
        aria-modal
        aria-label={title}
        tabIndex={-1}
        className={cx(
          "flex w-full max-w-440 flex-col gap-16 rounded-card border border-stroke bg-background p-22 text-text outline-none",
          className,
        )}
      >
        {children}
      </div>
    </div>
  );
}

/**
 * An alert or confirmation: a bold title, one sentence and its buttons on the right, as macOS draws `.alert`.
 */
export function AlertDialog({
  title,
  message,
  onCancel,
  children,
}: {
  title: string;
  message: string;
  onCancel: () => void;
  children: ReactNode;
}) {
  return (
    <Dialog title={title} role="alertdialog" onCancel={onCancel}>
      <div className="flex flex-col gap-6">
        <p className="m-0 text-heading font-bold">{title}</p>
        <p className="m-0 text-body text-muted">{message}</p>
      </div>
      <div className="flex justify-end gap-small">{children}</div>
    </Dialog>
  );
}
