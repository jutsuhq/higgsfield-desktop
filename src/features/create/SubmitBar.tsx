import { Info, Sparkles } from "lucide-react";

import type { ButtonView_Serialize, SubmitView_Serialize } from "../../bindings";
import { cx } from "../../design/cx";
import { Button } from "../../design/Styles";
import { useShortcut } from "../../lib/shortcuts/useShortcut";
import { submit } from "./createData";

/** The first thing to fix, else the price line, and Draft and Generate with their prices. */
export function SubmitBar({ view }: { view: SubmitView_Serialize }) {
  // A batch runs as drafts by default: Draft is the main button, and the shortcut presses it.
  const primaryIsDraft = view.primaryIsDraft && view.draft !== null;
  const primary = primaryIsDraft && view.draft ? view.draft : view.generate;
  // File › Generate is titled as the main button (`CreateForm.primarySubmit`).
  useShortcut(
    "generate",
    () => void submit(primaryIsDraft),
    primary.blocker === null && !view.isSubmitting,
    primary.title,
  );
  return (
    <div className="flex flex-col gap-medium">
      {view.blocker !== null ? (
        <p className="m-0 flex items-center gap-small text-small text-muted">
          <Info size={14} aria-hidden />
          {view.blocker}
        </p>
      ) : (
        view.costLine && (
          <div className="flex flex-col gap-2 tabular-nums">
            <span className="text-body font-bold text-text">{view.costLine.text}</span>
            <span className={cx("text-caption", view.costLine.isWarning ? "text-warning" : "text-faint")}>
              {view.costLine.detail}
            </span>
          </div>
        )
      )}
      {/* Credit prices and batch counts can outgrow a narrow panel: the main button then wraps to a row of its own. */}
      <div className="flex flex-wrap gap-small">
        {primaryIsDraft ? (
          <>
            <SubmitButton
              button={view.generate}
              isDraft={false}
              isPrimary={false}
              isSubmitting={view.isSubmitting}
            />
            {view.draft && (
              <SubmitButton button={view.draft} isDraft isPrimary isSubmitting={view.isSubmitting} />
            )}
          </>
        ) : (
          <>
            {view.draft && (
              <SubmitButton button={view.draft} isDraft isPrimary={false} isSubmitting={view.isSubmitting} />
            )}
            <SubmitButton button={view.generate} isDraft={false} isPrimary isSubmitting={view.isSubmitting} />
          </>
        )}
      </div>
    </div>
  );
}

function SubmitButton({
  button,
  isDraft,
  isPrimary,
  isSubmitting,
}: {
  button: ButtonView_Serialize;
  isDraft: boolean;
  isPrimary: boolean;
  isSubmitting: boolean;
}) {
  return (
    <Button
      variant={isPrimary ? "primary" : "secondary"}
      icon={isPrimary ? Sparkles : undefined}
      title={button.title}
      price={isPrimary ? (button.price ?? undefined) : undefined}
      detail={isPrimary ? undefined : (button.price ?? undefined)}
      className={isPrimary ? "grow" : "shrink-0"}
      disabled={button.blocker !== null || isSubmitting}
      onClick={() => void submit(isDraft)}
    />
  );
}
