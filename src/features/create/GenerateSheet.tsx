import { LoaderCircle, Sparkles, X } from "lucide-react";
import type { ReactNode } from "react";

import { Dialog } from "../../app/Dialog";
import { cx } from "../../design/cx";
import { RemoteThumbnail } from "../../design/RemoteThumbnail";
import { Spinner } from "../../design/Spinner";
import { Button, focusRing } from "../../design/Styles";
import { useMediaUrl } from "./createData";

/**
 * The frame Generate reference images and Create Character share (`GenerateSheetLayout`): a header with the title
 * and the close cross, the form on the left, what the requests made on the right, and the actions under both. While
 * the sheet is busy, the form and the cross wait.
 */
export function GenerateSheetLayout({
  title,
  subtitle,
  formWidth = "w-380",
  isBusy,
  closeHint,
  closesOnEscape = true,
  onClose,
  form,
  results,
  actions,
  children,
}: {
  title: string;
  subtitle: string;
  formWidth?: "w-380" | "w-400";
  isBusy: boolean;
  /** The cross's tooltip: why it waits, or "Close". */
  closeHint: string;
  /** Escape closes the sheet; off while a view over it takes Escape. */
  closesOnEscape?: boolean;
  onClose: () => void;
  form: ReactNode;
  results: ReactNode;
  actions: ReactNode;
  /** Views over the sheet: a result open large, a confirmation. */
  children?: ReactNode;
}) {
  return (
    <Dialog
      title={title}
      onCancel={() => {
        if (closesOnEscape && !isBusy) onClose();
      }}
      className="relative h-720 max-h-[90vh] max-w-1000 min-w-0 gap-large p-x-large"
    >
      <div className="flex items-start gap-medium">
        <div className="flex flex-1 flex-col gap-4">
          <h2 className="m-0 text-heading font-bold text-text">{title}</h2>
          <p className="m-0 text-body text-muted">{subtitle}</p>
        </div>
        <Button variant="icon" icon={X} title="Close" hint={closeHint} disabled={isBusy} onClick={onClose} />
      </div>
      <div className="flex min-h-0 flex-1 gap-x-large">
        <div
          aria-disabled={isBusy}
          className={cx("shrink-0 overflow-y-auto", formWidth, isBusy && "pointer-events-none opacity-55")}
        >
          {form}
        </div>
        <div className="w-px shrink-0 bg-stroke" />
        <div className="flex min-w-0 flex-1 flex-col">{results}</div>
      </div>
      <div className="flex items-center gap-small">{actions}</div>
      {children}
    </Dialog>
  );
}

/** A setting on one line of a form's well: its name on the left, the control on the right (`OptionRow`). */
export function SheetOptionRow({
  title,
  detail,
  drop,
  children,
}: {
  title: string;
  detail?: string | undefined;
  /** Files dropped on the row. */
  drop?: ((paths: string[]) => void) | undefined;
  children: ReactNode;
}) {
  return (
    <div
      className="flex min-h-44 items-center gap-medium border-b border-stroke py-6 last:border-b-0"
      onDragOver={(event) => {
        if (drop) event.preventDefault();
      }}
    >
      <span className="mr-auto flex flex-col gap-2">
        <span className="text-body text-text">{title}</span>
        {detail !== undefined && <span className="line-clamp-2 text-caption text-faint">{detail}</span>}
      </span>
      {children}
    </div>
  );
}

/** A picked image in an option row, with the cross that removes it (`RemovableThumb`). */
export function RemovableThumb({ path, onRemove }: { path: string; onRemove: () => void }) {
  const url = useMediaUrl(path);
  return (
    <span className="relative size-40 shrink-0">
      {url ? (
        <img src={url} alt="" className="size-40 rounded-small border border-stroke object-cover" />
      ) : (
        <span className="block size-40 rounded-small bg-glass" />
      )}
      <button
        type="button"
        aria-label="Remove"
        title="Remove"
        onClick={onRemove}
        className={cx(
          "absolute -top-8 -right-8 flex size-20 cursor-pointer items-center justify-center rounded-full",
          "border border-stroke bg-background text-text",
          focusRing,
        )}
      >
        <X aria-hidden size={10} strokeWidth={3} />
      </button>
    </span>
  );
}

/** Generate with its price, as on Create's Generate (`GenerateButtonLabel`). */
export function GenerateButton({
  isGenerating,
  price,
  isPrimary = true,
  hint,
  disabled,
  onClick,
}: {
  isGenerating: boolean;
  price: string | null;
  isPrimary?: boolean;
  hint: string;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <Button
      variant={isPrimary ? "primary" : "secondary"}
      icon={isGenerating ? LoaderCircle : Sparkles}
      title={isGenerating ? "Generating…" : isPrimary || price === null ? "Generate" : `Generate ${price}`}
      price={!isGenerating && isPrimary && price !== null ? price : undefined}
      hint={hint === "" ? undefined : hint}
      disabled={disabled}
      onClick={onClick}
    />
  );
}

/** The place of an image a running request will make. */
export function PendingTile({ label = "Image being made" }: { label?: string }) {
  return (
    <span
      role="img"
      aria-label={label}
      className="flex aspect-square w-full items-center justify-center rounded-small border border-dashed border-stroke bg-well"
    >
      <Spinner />
    </span>
  );
}

/** A remote image or its saved copy, filling a square. */
export function SquareThumb({ path, url }: { path: string | null; url: string }) {
  const local = useMediaUrl(path);
  return path !== null && local ? (
    <img src={local} alt="" className="absolute inset-0 size-full object-cover" />
  ) : (
    <RemoteThumbnail url={url} className="absolute inset-0 size-full" />
  );
}
