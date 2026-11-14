import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Layers, LayoutGrid, Pause, Play, Plus } from "lucide-react";
import { useEffect, useState } from "react";

import {
  commands,
  type JobCommand,
  type QueueBatch_Serialize,
  type QueueConfirmation,
  type QueueEntryModel_Serialize,
  type QueueRow_Serialize,
} from "../../bindings";
import { useShownQueueJob } from "../../app/agentShow";
import { AlertDialog } from "../../app/Dialog";
import { useToastStore } from "../../app/toastStore";
import { EmptyState, PageHeader, Pill, RenderProgress, SectionLabel } from "../../design/Components";
import { Button, Card, pagePadding } from "../../design/Styles";
import { queryKeys } from "../../lib/queryKeys";
import { useShortcut } from "../../lib/shortcuts/useShortcut";
import { useImagesSnapshot } from "../create/imageData";
import { ImageRequestsSection } from "./ImageRequestsSection";
import { JobRow, type RowHandlers } from "./JobRow";
import { useProgressChannel, useQueueSnapshot } from "./queueData";

/** Every job row of `entries`, batch members included. */
function rowsOf(entries: QueueEntryModel_Serialize[]): QueueRow_Serialize[] {
  return entries.flatMap((entry) => (entry.kind === "job" ? [entry.row] : entry.batch.rows));
}

/** A duplicate waiting for the user to agree to go over today's cap. */
interface PendingDuplicate {
  id: string;
  confirmation: QueueConfirmation;
}

/**
 * The Queue (QueueView.swift): what runs now, what didn't finish and what finished lately, as Rust's snapshot lists
 * them. A row clicked outside its buttons is selected for the Cancel Job and Retry shortcuts.
 */
export function QueuePage() {
  useProgressChannel();
  const snapshot = useQueueSnapshot().data;
  const imageRequests = useImagesSnapshot().data?.requests ?? [];
  const navigate = useNavigate();
  const client = useQueryClient();
  const showQueued = useToastStore((state) => state.showQueued);
  // An agent's `show` of a job selects its row (`showInQueue`): the one asked before the Queue opened, then each later one.
  const [selectedId, setSelectedId] = useState<string | null>(() => useShownQueueJob.getState().id);
  useEffect(() => {
    useShownQueueJob.getState().set(null);
    return useShownQueueJob.subscribe((state) => {
      if (state.id === null) return;
      setSelectedId(state.id);
      state.set(null);
    });
  }, []);
  const [isDuplicating, setIsDuplicating] = useState(false);
  const [pendingDuplicate, setPendingDuplicate] = useState<PendingDuplicate | null>(null);
  const [isConfirmingClear, setIsConfirmingClear] = useState(false);

  const run = async (action: () => Promise<unknown>) => {
    try {
      await action();
    } finally {
      await client.invalidateQueries({ queryKey: queryKeys.queue.all });
    }
  };

  const perform = (id: string, command: JobCommand) => run(() => commands.queueJobCommand(id, command));

  const duplicate = (id: string, overCapApproved: boolean) => {
    setIsDuplicating(true);
    void run(async () => {
      const outcome = await commands.queueDuplicate(id, overCapApproved);
      if (outcome.kind === "queued") showQueued(outcome.notice);
      if (outcome.kind === "confirm") setPendingDuplicate({ id, confirmation: outcome.confirmation });
    }).finally(() => {
      setIsDuplicating(false);
    });
  };

  const goToCreate = () => {
    void navigate({ to: "/create" });
  };

  const allRows = snapshot ? rowsOf([...snapshot.active, ...snapshot.problems, ...snapshot.finished]) : [];
  const selected = allRows.find((row) => row.model.id === selectedId);
  const selectedRetry = selected?.commands.retry ?? null;
  useShortcut(
    "cancelJob",
    () => {
      if (selected) void perform(selected.model.id, "cancel");
    },
    selected?.commands.canCancel ?? false,
  );
  useShortcut(
    "retryJob",
    () => {
      if (selected && selectedRetry) void perform(selected.model.id, selectedRetry);
    },
    selectedRetry !== null,
    selected?.commands.retryTitle ?? null,
  );

  const handlers: RowHandlers = {
    perform,
    duplicate: isDuplicating
      ? undefined
      : (id) => {
          duplicate(id, false);
        },
    selectedId,
    toggleSelection: (id) => {
      setSelectedId((current) => (current === id ? null : id));
    },
  };

  const activeCount = snapshot?.activeCount ?? 0;
  const problemCount = snapshot ? rowsOf(snapshot.problems).length : 0;
  const finishedCount = snapshot ? rowsOf(snapshot.finished).length : 0;
  const isChangingAll = snapshot?.isChangingAll ?? false;

  return (
    <div className={pagePadding}>
      <div className="flex flex-col gap-small">
        <PageHeader
          title="Queue"
          subtitle="Jobs keep running in the background and resume after restart."
          trailing={
            <div className="flex items-center gap-small">
              {snapshot && <Pill text={snapshot.slotsSummary} dot="accent" isPulsing={snapshot.isBusy} />}
              <div className="flex items-center gap-small">
                <Button
                  variant="icon"
                  icon={Pause}
                  isIconFilled
                  title="Pause all"
                  hint="Pause all"
                  disabled={isChangingAll || !(snapshot?.canPauseAll ?? false)}
                  onClick={() => void run(() => commands.queuePauseAll())}
                />
                <Button
                  variant="icon"
                  icon={Play}
                  isIconFilled
                  title="Resume all"
                  hint="Resume all"
                  disabled={isChangingAll || !(snapshot?.canResumeAll ?? false)}
                  onClick={() => void run(() => commands.queueResumeAll())}
                />
              </div>
              <Button variant="secondary" icon={Plus} title="New job" onClick={goToCreate} />
            </div>
          }
        />
        <SectionLabel title="Active" detail={String(activeCount)} />
        {snapshot && snapshot.active.length === 0 && imageRequests.length === 0 && (
          <Card>
            <EmptyState
              icon={Layers}
              title="Nothing running"
              detail="Start a draft from Create. Jobs keep running when the window is closed."
              action={<Button variant="secondary" title="Go to Create" onClick={goToCreate} />}
            />
          </Card>
        )}
        <EntryList entries={snapshot?.active ?? []} handlers={handlers} label="Active jobs" />
        <ImageRequestsSection requests={imageRequests} />

        {problemCount > 0 && (
          <>
            <div className="flex items-baseline gap-medium pt-18">
              <div className="flex-1">
                <SectionLabel title="Didn't finish" detail={String(problemCount)} />
              </div>
              <Button
                variant="textLink"
                title="Clear all"
                hint="Remove all from Queue"
                onClick={() => {
                  setIsConfirmingClear(true);
                }}
              />
            </div>
            <EntryList
              entries={snapshot?.problems ?? []}
              handlers={{
                ...handlers,
                remove: (id) => void run(() => commands.queueRemoveProblem(id)),
              }}
              label="Jobs that didn't finish"
            />
          </>
        )}

        <div className="pt-18">
          <SectionLabel title="Finished" detail={String(finishedCount)} />
        </div>
        {finishedCount === 0 && (
          <p className="m-0 py-small text-control text-faint">
            Finished jobs appear here and in the Library.
          </p>
        )}
        <EntryList entries={snapshot?.finished ?? []} handlers={handlers} label="Finished jobs" />
      </div>

      {isConfirmingClear && (
        <AlertDialog
          title="Clear the jobs that didn't finish?"
          message="They leave the Queue. What they cost still counts in today's spending."
          onCancel={() => {
            setIsConfirmingClear(false);
          }}
        >
          <Button
            variant="secondary"
            title="Cancel"
            onClick={() => {
              setIsConfirmingClear(false);
            }}
          />
          <Button
            variant="secondary"
            destructive
            title="Clear all"
            onClick={() => {
              setIsConfirmingClear(false);
              void run(() => commands.queueClearProblems());
            }}
          />
        </AlertDialog>
      )}

      {pendingDuplicate && (
        <AlertDialog
          title={pendingDuplicate.confirmation.title}
          message={pendingDuplicate.confirmation.message}
          onCancel={() => {
            setPendingDuplicate(null);
          }}
        >
          <Button
            variant="secondary"
            title="Cancel"
            onClick={() => {
              setPendingDuplicate(null);
            }}
          />
          <Button
            variant="primary"
            title={pendingDuplicate.confirmation.confirmTitle}
            onClick={() => {
              setPendingDuplicate(null);
              duplicate(pendingDuplicate.id, true);
            }}
          />
        </AlertDialog>
      )}
    </div>
  );
}

/** Jobs on their own, and batches as a header over their jobs (`QueueEntryList`). */
function EntryList({
  entries,
  handlers,
  label,
}: {
  entries: QueueEntryModel_Serialize[];
  handlers: RowHandlers;
  label: string;
}) {
  if (entries.length === 0) return null;
  return (
    <div role="listbox" aria-label={label} className="flex flex-col gap-small">
      {entries.map((entry) =>
        entry.kind === "job" ? (
          <JobRow key={entry.row.model.id} row={entry.row} handlers={handlers} />
        ) : (
          <div
            key={entry.batch.id}
            role="group"
            aria-label={entry.batch.title}
            className="flex flex-col gap-small"
          >
            <BatchHeader batch={entry.batch} />
            {entry.batch.rows.map((row) => (
              <div key={row.model.id} className="pl-large">
                <JobRow row={row} handlers={handlers} />
              </div>
            ))}
          </div>
        ),
      )}
    </div>
  );
}

/** A batch's title, progress and total price over its jobs, and Cancel All (`BatchHeader`). */
function BatchHeader({ batch }: { batch: QueueBatch_Serialize }) {
  const [isCanceling, setIsCanceling] = useState(false);
  /** Asked before Cancel All once a job is with Higgsfield. */
  const [confirmation, setConfirmation] = useState<QueueConfirmation | null>(null);
  // Rust cancels the jobs as they are when it runs, and says through the connection alert which ones it refused.
  const cancelAll = (approved: boolean) => {
    setIsCanceling(true);
    commands
      .queueCancelAll(batch.id, approved)
      .then((question) => {
        setConfirmation(question);
      })
      .catch(() => undefined)
      .finally(() => {
        setIsCanceling(false);
      });
  };
  return (
    <Card className="flex flex-col gap-6 p-medium">
      <div className="flex items-center gap-small">
        <LayoutGrid aria-hidden size={13} className="shrink-0 text-accent" />
        <span className="mr-auto truncate text-button font-semibold text-text">{batch.title}</span>
        <span className="shrink-0 text-small font-medium text-faint tabular-nums">
          {[batch.progressText, batch.totalCostText].filter((part) => part !== null).join(" · ")}
        </span>
        {batch.canCancelAll && (
          <Button
            variant="secondary"
            title="Cancel All"
            hint="Cancel unstarted jobs"
            disabled={isCanceling}
            onClick={() => {
              cancelAll(false);
            }}
          />
        )}
      </div>
      {batch.isActive && batch.progress !== null && <RenderProgress value={batch.progress} />}
      {confirmation && (
        <AlertDialog
          title={confirmation.title}
          message={confirmation.message}
          onCancel={() => {
            setConfirmation(null);
          }}
        >
          <Button
            variant="secondary"
            title="Keep Them"
            onClick={() => {
              setConfirmation(null);
            }}
          />
          <Button
            variant="secondary"
            destructive
            title={confirmation.confirmTitle}
            onClick={() => {
              setConfirmation(null);
              cancelAll(true);
            }}
          />
        </AlertDialog>
      )}
    </Card>
  );
}
