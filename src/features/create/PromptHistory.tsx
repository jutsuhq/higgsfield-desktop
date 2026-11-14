import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { History, Star, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { commands, type PromptRow_Serialize } from "../../bindings";
import { useToastStore } from "../../app/toastStore";
import { cx } from "../../design/cx";
import { HoverHint } from "../../design/HoverHint";
import { SearchField } from "../../design/Pickers";
import { Button, focusRing } from "../../design/Styles";
import { queryKeys } from "../../lib/queryKeys";
import { errorText, showError } from "./createData";

/** The cross next to a prompt's title: `clear` empties the prompt. */
export function PromptClearButton({ clear }: { clear: () => void }) {
  return <Button variant="icon" icon={X} title="Clear Prompt" hint="Clear prompt" onClick={clear} />;
}

/**
 * Replaces the video form's prompt, with Undo when that loses an edited one: a prompt picked from the history, or
 * Clear prompt (`""`).
 */
export async function replacePrompt(text: string): Promise<void> {
  try {
    if (await commands.createReplacePrompt(text)) {
      useToastStore.getState().showDraftReplaced(() => void commands.createUndoReplacement());
    }
  } catch (error) {
    showError("Can't change the form", error);
  }
}

/**
 * The clock button next to the video prompt (`PromptHistoryButton`): opens the history of submitted prompts; picking
 * one replaces the prompt. Closes on a pick, Esc or a click elsewhere.
 */
export function PromptHistoryButton() {
  const [isOpen, setIsOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!isOpen) return;
    const close = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setIsOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setIsOpen(false);
    };
    document.addEventListener("pointerdown", close);
    window.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [isOpen]);
  return (
    <div ref={root} className="relative">
      <Button
        variant="icon"
        icon={History}
        title="Prompt History"
        hint="Reuse an earlier prompt"
        aria-expanded={isOpen}
        onClick={() => {
          setIsOpen((open) => !open);
        }}
      />
      {isOpen && (
        <PromptHistoryPicker
          onPick={(text) => {
            setIsOpen(false);
            void replacePrompt(text);
          }}
        />
      )}
    </div>
  );
}

function PromptHistoryPicker({ onPick }: { onPick: (text: string) => void }) {
  const [query, setQuery] = useState("");
  const [failure, setErrorText] = useState<string | null>(null);
  const history = useQuery({
    queryKey: queryKeys.prompts(query),
    queryFn: () => commands.promptsHistory(query),
    staleTime: 0,
    placeholderData: keepPreviousData,
  });
  const setStarred = (row: PromptRow_Serialize) => {
    commands.promptsSetStarred(row.text, !row.isStarred).then(
      () => {
        setErrorText(null);
        void history.refetch();
      },
      (error: unknown) => {
        setErrorText(errorText(error));
      },
    );
  };
  const view = history.data;
  return (
    <div
      role="dialog"
      aria-label="Prompt History"
      className="absolute top-0 left-full z-40 ml-small flex h-360 w-380 flex-col gap-medium rounded-card border border-glass-stroke bg-background p-large shadow-toast"
    >
      <SearchField value={query} onChange={setQuery} prompt="Search prompts" />
      {view?.emptyText != null ? (
        <p className="m-auto text-body text-muted">{view.emptyText}</p>
      ) : (
        <ul className="m-0 flex flex-1 list-none flex-col gap-x-small overflow-y-auto p-0">
          {view?.rows.map((row) => (
            <li key={row.text} className="flex items-start gap-small rounded-field border border-stroke bg-well p-small">
              <HoverHint text="Use this prompt" className="min-w-0 flex-1">
                <button
                  type="button"
                  onClick={() => {
                    onPick(row.text);
                  }}
                  className={cx("flex w-full cursor-pointer flex-col gap-2 text-left", focusRing)}
                >
                  <span className="line-clamp-2 text-body text-text">{row.text}</span>
                  {row.lastUsed !== null && <span className="text-caption text-faint">{row.lastUsed}</span>}
                </button>
              </HoverHint>
              <Button
                variant="icon"
                icon={Star}
                isIconFilled={row.isStarred}
                title={row.isStarred ? "Unstar" : "Star"}
                hint={row.isStarred ? "Unstar" : "Star"}
                className={row.isStarred ? "text-accent!" : "text-muted!"}
                onClick={() => {
                  setStarred(row);
                }}
              />
            </li>
          ))}
        </ul>
      )}
      {failure !== null && <p className="m-0 text-caption text-danger">{failure}</p>}
    </div>
  );
}
