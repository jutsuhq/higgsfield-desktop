import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { ListOrdered, Sparkles } from "lucide-react";
import { useState } from "react";

import { commands, events, type QueueConfirmation, type Resolution, type SubmitOutcome } from "../../bindings";
import { AlertDialog, Dialog } from "../../app/Dialog";
import { useToastStore } from "../../app/toastStore";
import { SectionLabel } from "../../design/Components";
import { SegmentedControl } from "../../design/Pickers";
import { Spinner } from "../../design/Spinner";
import { Button } from "../../design/Styles";
import { useInvalidateOn } from "../../lib/events";
import { queryKeys } from "../../lib/queryKeys";
import { usePageCommand } from "../../lib/shortcuts/pageCommands";

/** A question before going on, or a reason it can't; asked by the Library's follow-ups. */
export interface Question {
  title: string;
  message: string;
  /** The confirm button; none for a plain notice. */
  confirmTitle?: string;
  again?: () => void;
}

/** The question the Library's follow-ups ask, and the dialog that asks it. */
/** `blockedTitle` heads the alert when Rust says the job can't be queued yet. */
export function useQuestion(blockedTitle = "Can't queue this yet") {
  const [question, setQuestion] = useState<Question | null>(null);
  const confirm = (confirmation: QueueConfirmation, again: () => void) => {
    setQuestion({ title: confirmation.title, message: confirmation.message, confirmTitle: confirmation.confirmTitle, again });
  };
  const dialog = question && (
    <AlertDialog
      title={question.title}
      message={question.message}
      onCancel={() => {
        setQuestion(null);
      }}
    >
      <Button
        variant="secondary"
        title={question.again ? "Cancel" : "OK"}
        onClick={() => {
          setQuestion(null);
        }}
      />
      {question.again && question.confirmTitle !== undefined && (
        <Button
          variant="primary"
          title={question.confirmTitle}
          onClick={() => {
            const again = question.again;
            setQuestion(null);
            again?.();
          }}
        />
      )}
    </AlertDialog>
  );
  /** What a render answered: the toast when queued, the question when it needs one, the reason it can't. */
  const handle = (outcome: SubmitOutcome, again: () => void) => {
    switch (outcome.kind) {
      case "queued":
        useToastStore.getState().showQueued(outcome.notice);
        return;
      case "confirm":
        confirm(outcome.confirmation, again);
        return;
      case "blocked":
        setQuestion({ title: blockedTitle, message: outcome.reason });
        return;
      case "busy":
      case "failed":
        return;
    }
  };
  return { dialog, confirm, handle, ask: setQuestion };
}

/** Render final of draft `id` at `resolution`, asking about the cap first. */
export function renderFinal(
  id: string,
  resolution: Resolution,
  handle: (outcome: SubmitOutcome, again: () => void) => void,
  approved = false,
): Promise<void> {
  return commands.libraryRenderFinal(id, resolution, approved).then((outcome) => {
    handle(outcome, () => void renderFinal(id, resolution, handle, true));
  });
}

/** The "Render final" box of a draft's page (`RenderFinalSection.swift`). */
export function RenderFinalSection({ id }: { id: string }) {
  const navigate = useNavigate();
  const [resolution, setResolution] = useState<Resolution | null>(null);
  // A final leaving or joining the queue changes the button: Final queued · View in Queue.
  useInvalidateOn(events.queueChanged, [[...queryKeys.library.all, "final", id]]);
  const question = useQuestion();
  const section = useQuery({
    queryKey: [...queryKeys.library.all, "final", id, resolution],
    queryFn: () => commands.libraryFinalSection(id, resolution),
    placeholderData: keepPreviousData,
  }).data;
  // Library › Render Final on this page presses the button (`LibraryDetail.menuCommands`).
  usePageCommand(
    "renderFinal",
    () => {
      if (section) void renderFinal(id, section.selected, question.handle);
    },
    section?.isConnected === true && !section.isQueued,
    "Render Final",
  );
  if (!section) return null;
  return (
    <section aria-label="Render final" className="flex flex-col gap-small">
      <SectionLabel title="Render final" detail={section.detail ?? undefined} />
      {section.options.length > 1 && (
        <SegmentedControl
          isCompact
          value={section.selected}
          options={section.options.map((option) => ({ value: option.value, title: option.title }))}
          onChange={(value) => {
            setResolution(value);
          }}
        />
      )}
      {section.window !== null && <p className="m-0 text-caption text-muted">{section.window}</p>}
      {section.isQueued ? (
        <Button
          variant="secondary"
          icon={ListOrdered}
          title="Final queued · View in Queue"
          onClick={() => void navigate({ to: "/queue" })}
        />
      ) : section.isConnected ? (
        <Button
          variant="primary"
          icon={Sparkles}
          title={section.buttonTitle}
          hint={section.help ?? undefined}
          price={section.price ?? undefined}
          onClick={() => void renderFinal(id, section.selected, question.handle)}
        />
      ) : (
        <p className="m-0 text-caption text-muted">Connect Higgsfield in Settings</p>
      )}
      {question.dialog}
    </section>
  );
}

/** Render finals for starred (`RenderFinalsSheet.swift`): one resolution for the starred drafts among `ids`. */
export function RenderFinalsSheet({ ids, onClose }: { ids: string[]; onClose: () => void }) {
  const navigate = useNavigate();
  const [resolution, setResolution] = useState<Resolution | null>(null);
  const question = useQuestion("Can't queue the finals");
  const sheet = useQuery({
    queryKey: [...queryKeys.library.all, "finals", ids, resolution],
    queryFn: () => commands.libraryFinalsSheet(ids, resolution),
    placeholderData: keepPreviousData,
  }).data;
  const render = (approved: boolean) => {
    if (!sheet) return;
    void commands.libraryRenderFinals(ids, sheet.selected, approved).then((outcome) => {
      if (outcome.kind === "queued") onClose();
      question.handle(outcome, () => {
        render(true);
      });
    });
  };
  return (
    <Dialog title="Render finals for starred" onCancel={onClose}>
      <div className="flex flex-col gap-6">
        <p className="m-0 text-heading font-bold">Render finals for starred</p>
        {sheet && <p className="m-0 text-body text-muted">{sheet.subtitle}</p>}
      </div>
      {!sheet ? (
        <Spinner />
      ) : (
        <>
          <SectionLabel title="Resolution" detail={sheet.detail ?? undefined} />
          {sheet.options.length > 1 && (
            <SegmentedControl
              isCompact
              value={sheet.selected}
              options={sheet.options.map((option) => ({ value: option.value, title: option.title }))}
              onChange={(value) => {
                setResolution(value);
              }}
            />
          )}
          <p className="m-0 text-caption text-muted">Each final keeps its draft&apos;s inputs and is linked to it.</p>
          {sheet.queuedNote !== null && <p className="m-0 text-caption text-muted">{sheet.queuedNote}</p>}
          {!sheet.isConnected && <p className="m-0 text-caption text-muted">Connect Higgsfield in Settings</p>}
          <div className="flex justify-end gap-small">
            <Button variant="secondary" title="Cancel" onClick={onClose} />
            {sheet.count === 0 ? (
              <Button
                variant="primary"
                title="View in Queue"
                onClick={() => {
                  onClose();
                  void navigate({ to: "/queue" });
                }}
              />
            ) : (
              <Button
                variant="primary"
                title={sheet.buttonTitle}
                price={sheet.price ?? undefined}
                disabled={!sheet.isConnected}
                onClick={() => {
                  render(false);
                }}
              />
            )}
          </div>
        </>
      )}
      {question.dialog}
    </Dialog>
  );
}

/** Score Variants (`ScoreVariants.swift`): the price of scoring `ids`, then the scoring. */
export function ScoreSheet({ ids, onClose }: { ids: string[]; onClose: () => void }) {
  const [consents, setConsents] = useState(false);
  const question = useQuestion();
  const queryClient = useQueryClient();
  const sheet = useQuery({
    queryKey: [...queryKeys.library.all, "score", ids],
    queryFn: () => commands.libraryScoreSheet(ids),
  }).data;
  const score = (approved: boolean) => {
    void commands.libraryScore(ids, approved).then((outcome) => {
      switch (outcome.kind) {
        case "started":
          void queryClient.invalidateQueries({ queryKey: queryKeys.library.all });
          onClose();
          return;
        case "confirm":
          question.confirm(outcome.confirmation, () => {
            score(true);
          });
          return;
        case "blocked":
          question.ask({ title: "Can't score these yet", message: outcome.reason });
          return;
      }
    });
  };
  const title = sheet?.title ?? "Score Variants";
  return (
    <Dialog title={title} onCancel={onClose}>
      <p className="m-0 text-heading font-bold">{title}</p>
      {!sheet ? (
        <Spinner label="Asking Higgsfield for the price…" />
      ) : (
        <>
          {sheet.notice !== null && <p className="m-0 text-body text-muted">{sheet.notice}</p>}
          <p className="m-0 text-body">{sheet.priceText}</p>
          <p className="m-0 text-caption text-muted">
            Scores are kept on each result. Picking the winner stays up to you.
          </p>
          {sheet.needsConsent && (
            <label className="flex items-center gap-small text-body">
              <input
                type="checkbox"
                checked={consents}
                onChange={(event) => {
                  setConsents(event.currentTarget.checked);
                }}
              />
              {sheet.consentText}
            </label>
          )}
          {sheet.blocker !== null && <p className="m-0 text-caption text-danger">{sheet.blocker}</p>}
          <div className="flex justify-end gap-small">
            <Button variant="secondary" title="Cancel" onClick={onClose} />
            <Button
              variant="primary"
              title={sheet.buttonTitle}
              price={sheet.priceTag ?? undefined}
              disabled={sheet.blocker !== null || (sheet.needsConsent && !consents)}
              onClick={() => {
                score(false);
              }}
            />
          </div>
        </>
      )}
      {question.dialog}
    </Dialog>
  );
}

/** Copy Videos: the clipboard gets the files' paths, one per line; there is no file clipboard here. */
export async function copyVideos(ids: string[]): Promise<void> {
  const files = await commands.libraryFiles(ids);
  if (files.length > 0) await navigator.clipboard.writeText(files.join("\n"));
}
