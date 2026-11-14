import { useQuery } from "@tanstack/react-query";
import { useNavigate, useParams } from "@tanstack/react-router";
import { ArrowLeft, CircleX, Flame, Sparkles, Trash2, Trophy } from "lucide-react";
import { useEffect, useState } from "react";

import { commands, type BatchCellView } from "../../bindings";
import { EmptyState, PageHeader } from "../../design/Components";
import { Spinner } from "../../design/Spinner";
import { Button } from "../../design/Styles";
import { isEditingText } from "../../design/textFocus";
import { queryKeys } from "../../lib/queryKeys";
import { RenderFinalsSheet, ScoreSheet } from "../library/followUps";
import { DeleteDialog } from "../library/LibraryDialogs";
import { ComparePlayer } from "./ComparePlayer";

/** A batch opened as a comparison (`BatchComparison.swift`): every result on one clock, each with its label. */
export function BatchPage() {
  const id = useParams({ strict: false }).id ?? "";
  const navigate = useNavigate();
  const query = useQuery({
    queryKey: [...queryKeys.library.all, "batch", id],
    queryFn: () => commands.libraryBatch(id),
  });
  const view = query.data;
  const [sheet, setSheet] = useState<"score" | "finals" | "delete" | null>(null);
  const back = () => void navigate({ to: "/library" });

  useEffect(() => {
    if (sheet !== null) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || isEditingText()) return;
      if (event.key !== "Escape") return;
      event.preventDefault();
      back();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  });

  const backButton = (
    <Button variant="secondary" icon={ArrowLeft} title="Library" hint="Back to Library (Esc)" onClick={back} />
  );
  if (query.isError) {
    return (
      <div className="flex h-full flex-col px-page pt-page-top">
        <PageHeader title="Library" trailing={backButton} />
        <EmptyState icon={CircleX} title="This batch is no longer in the library." />
      </div>
    );
  }
  if (!view) {
    return (
      <div className="flex h-full flex-col px-page pt-page-top">
        <PageHeader title="Library" trailing={backButton} />
        <Spinner />
      </div>
    );
  }
  const onDisk = view.cells.filter((cell) => cell.file.exists);
  const score = view.score.availability;
  return (
    <div className="flex min-h-full flex-col gap-medium px-page pt-page-top pb-page">
      <PageHeader
        title={view.title}
        subtitle={view.subtitle}
        trailing={
          <div className="flex items-center gap-small">
            {backButton}
            {score.kind !== "hidden" && (
              <Button
                variant="secondary"
                icon={Flame}
                title={view.score.title}
                hint={score.kind === "disabled" ? score.reason : (view.score.blocker ?? "Predict virality")}
                disabled={score.kind === "disabled" || view.score.blocker !== null || view.score.isRunning}
                onClick={() => {
                  setSheet("score");
                }}
              />
            )}
            {view.canRenderFinals && (
              <Button
                variant="secondary"
                icon={Sparkles}
                title="Render Finals for Starred…"
                hint={view.finalsHelp}
                disabled={!view.hasStarredDrafts}
                onClick={() => {
                  setSheet("finals");
                }}
              />
            )}
            <Button
              variant="secondary"
              icon={Trash2}
              destructive
              title="Delete Unstarred…"
              hint="Delete unstarred"
              disabled={view.unstarredIds.length === 0}
              onClick={() => {
                setSheet("delete");
              }}
            />
          </div>
        }
      />
      {onDisk.length === 0 ? (
        <EmptyState icon={CircleX} title="File not found" />
      ) : (
        <ComparePlayer
          tracks={onDisk.map((cell) => ({ id: cell.id, labels: [cell.label], url: cell.file.url ?? null }))}
          layout="sideBySide"
          columns={view.columns}
          isActive={sheet === null}
        />
      )}
      <div className="grid gap-small" style={{ gridTemplateColumns: `repeat(${String(Math.max(view.columns, 1))}, minmax(0, 1fr))` }}>
        {view.cells.map((cell) => (
          <CellRow key={cell.id} cell={cell} />
        ))}
      </div>
      {sheet === "score" && (
        <ScoreSheet
          ids={view.memberIds}
          onClose={() => {
            setSheet(null);
          }}
        />
      )}
      {sheet === "finals" && (
        <RenderFinalsSheet
          ids={view.memberIds}
          onClose={() => {
            setSheet(null);
          }}
        />
      )}
      {sheet === "delete" && (
        <DeleteDialog
          ids={view.unstarredIds}
          onDone={() => {
            setSheet(null);
          }}
        />
      )}
    </div>
  );
}

/** Under a cell: its label, its score, and Pick winner for a draft or Open. */
function CellRow({ cell }: { cell: BatchCellView }) {
  const navigate = useNavigate();
  const open = () => void navigate({ to: "/library/result/$id", params: { id: cell.id } });
  const { virality } = cell;
  return (
    <div className="flex min-w-0 items-center gap-x-small">
      <span className="truncate text-body font-semibold text-text">{cell.label}</span>
      {virality.isScoring ? (
        <Spinner label="Scoring" />
      ) : virality.badge !== null ? (
        <span
          title={virality.help ?? undefined}
          aria-label={virality.accessibilityLabel ?? undefined}
          className="rounded-full bg-ink-wash px-8 py-3 text-caption font-semibold tabular-nums"
        >
          {virality.badge}
        </span>
      ) : virality.reportUrl !== null ? (
        <a href={virality.reportUrl} target="_blank" rel="noreferrer" title="Open report on Higgsfield" className="text-caption text-accent">
          Open report
        </a>
      ) : virality.failure !== null ? (
        <span className="text-caption text-danger" title={virality.failure}>
          {virality.failure}
        </span>
      ) : null}
      <span className="ml-auto">
        {cell.canRenderFinal ? (
          <Button variant="secondary" icon={Trophy} title="Pick winner" hint="Open to render final" onClick={open} />
        ) : (
          <Button variant="secondary" title="Open" hint="Open this result" onClick={open} />
        )}
      </span>
    </div>
  );
}
