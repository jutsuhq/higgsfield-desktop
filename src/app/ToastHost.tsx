import { useNavigate, useRouterState } from "@tanstack/react-router";
import { CircleCheck, Undo2, X, type LucideIcon } from "lucide-react";
import { useEffect } from "react";

import { Button } from "../design/Styles";
import { draftReplacementDurationMs, queuedNoticeDurationMs, useToastStore } from "./toastStore";

/**
 * `queuedToast()`: what the last submit queued, at the bottom of every page for a moment, with a way to the Queue
 * when it isn't on screen already; otherwise the draft a replacement lost, with Undo. Each toast is announced.
 */
export function ToastHost() {
  const { queuedNotice, draftReplacement, dismissQueued, dismissDraftReplacement } = useToastStore();
  const navigate = useNavigate();
  const isOnQueue = useRouterState({ select: (state) => state.location.pathname === "/queue" });

  useExpiry(queuedNotice?.id, queuedNoticeDurationMs, () => {
    if (useToastStore.getState().queuedNotice?.id === queuedNotice?.id) dismissQueued();
  });
  useExpiry(draftReplacement?.id, draftReplacementDurationMs, () => {
    if (useToastStore.getState().draftReplacement?.id === draftReplacement?.id) dismissDraftReplacement();
  });

  return (
    <div
      role="status"
      aria-live="polite"
      className="pointer-events-none absolute inset-x-0 bottom-0 flex justify-center"
    >
      {queuedNotice ? (
        <Toast
          key={queuedNotice.id}
          icon={CircleCheck}
          text={queuedNotice.text}
          action={
            isOnQueue
              ? undefined
              : {
                  title: "View in Queue",
                  run: () => {
                    void navigate({ to: "/queue" });
                  },
                }
          }
          onDismiss={dismissQueued}
        />
      ) : draftReplacement ? (
        <Toast
          key={draftReplacement.id}
          icon={Undo2}
          text="Draft replaced"
          action={{ title: "Undo", run: draftReplacement.undo }}
          onDismiss={dismissDraftReplacement}
        />
      ) : null}
    </div>
  );
}

/** Calls `expire` `duration` ms after a toast with `id` appears, unless a newer one replaced it first. */
function useExpiry(id: string | undefined, duration: number, expire: () => void) {
  useEffect(() => {
    if (id === undefined) return;
    const timer = window.setTimeout(expire, duration);
    return () => {
      window.clearTimeout(timer);
    };
    // `expire` reads the store when it fires; only a new toast restarts the timer.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id, duration]);
}

/** `action` runs after the toast is dismissed. */
export function Toast({
  icon: Icon,
  text,
  action,
  onDismiss,
}: {
  icon: LucideIcon;
  text: string;
  action?: { title: string; run: () => void } | undefined;
  onDismiss: () => void;
}) {
  return (
    <div className="pointer-events-auto mb-x-large flex animate-toast-in items-center gap-medium rounded-full border-[0.5px] border-glass-stroke bg-glass py-small pr-small pl-large shadow-toast [background-image:linear-gradient(var(--color-toast),var(--color-toast))]">
      <Icon aria-hidden size={15} className="shrink-0 text-accent" />
      <span className="truncate text-control font-semibold text-text tabular-nums">{text}</span>
      {action && (
        <Button
          variant="textLink"
          title={action.title}
          onClick={() => {
            onDismiss();
            action.run();
          }}
        />
      )}
      <Button variant="icon" title="Dismiss" icon={X} hint="Dismiss" onClick={onDismiss} />
    </div>
  );
}
