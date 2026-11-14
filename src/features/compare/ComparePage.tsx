import { useQuery } from "@tanstack/react-query";
import { useNavigate, useParams } from "@tanstack/react-router";
import { ArrowLeft, ArrowLeftRight, CircleX } from "lucide-react";
import { useEffect, useState } from "react";

import { commands, type CompareLayout, type CompareView } from "../../bindings";
import { EmptyState, PageHeader } from "../../design/Components";
import { SegmentedControl } from "../../design/Pickers";
import { Spinner } from "../../design/Spinner";
import { Button } from "../../design/Styles";
import { isEditingText } from "../../design/textFocus";
import { queryKeys } from "../../lib/queryKeys";
import { ComparePlayer } from "./ComparePlayer";

/** Compare (`LibraryCompare.swift`) of the results in the route, in that order. */
export function ComparePage() {
  const ids = (useParams({ strict: false }).ids ?? "").split(",").filter((id) => id !== "");
  const view = useQuery({
    queryKey: [...queryKeys.library.all, "compare", ids],
    queryFn: () => commands.libraryCompare(ids),
  });
  return <CompareScreen view={view.data} isError={view.isError} />;
}

/** Compare with Original: the result beside the version it was made from. */
export function CompareOriginalPage() {
  const id = useParams({ strict: false }).id ?? "";
  const view = useQuery({
    queryKey: [...queryKeys.library.all, "compareOriginal", id],
    queryFn: () => commands.libraryCompareOriginal(id),
  });
  return <CompareScreen view={view.data} isError={view.isError} />;
}

function CompareScreen({ view, isError }: { view: CompareView | undefined; isError: boolean }) {
  const navigate = useNavigate();
  const [layout, setLayout] = useState<CompareLayout>("sideBySide");
  const [isSwapped, setIsSwapped] = useState(false);
  const back = () => {
    window.history.back();
  };
  const canSwap = view?.tracks.length === 2;

  // Esc goes back; ← and → swap the sides.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || isEditingText() || event.ctrlKey || event.altKey || event.metaKey) return;
      if (event.key === "Escape") back();
      else if ((event.key === "ArrowLeft" || event.key === "ArrowRight") && canSwap) setIsSwapped((swapped) => !swapped);
      else return;
      event.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  });

  const header = (
    <PageHeader
      title="Compare"
      trailing={
        <div className="flex items-center gap-small">
          <Button variant="secondary" icon={ArrowLeft} title="Back" hint="Back (Esc)" onClick={back} />
          {view && view.layouts.length > 1 && (
            <SegmentedControl
              isCompact
              value={layout}
              options={view.layouts.map((option) => ({ value: option.value, title: option.title }))}
              onChange={setLayout}
            />
          )}
          {canSwap && (
            <Button
              variant="secondary"
              icon={ArrowLeftRight}
              title="Swap Sides"
              hint="Swap sides (← →)"
              onClick={() => {
                setIsSwapped((swapped) => !swapped);
              }}
            />
          )}
        </div>
      }
    />
  );
  if (isError) {
    return (
      <div className="flex h-full flex-col px-page pt-page-top">
        {header}
        <EmptyState
          icon={CircleX}
          title="These results can't be compared."
          action={<Button variant="secondary" title="Library" onClick={() => void navigate({ to: "/library" })} />}
        />
      </div>
    );
  }
  if (!view) {
    return (
      <div className="flex h-full flex-col px-page pt-page-top">
        {header}
        <Spinner />
      </div>
    );
  }
  const tracks = isSwapped ? [...view.tracks].reverse() : view.tracks;
  return (
    <div className="flex min-h-full flex-col gap-medium px-page pt-page-top pb-page">
      {header}
      <ComparePlayer
        key={tracks.map((track) => track.id).join()}
        tracks={tracks.map((track) => ({ id: track.id, labels: track.labels, url: track.file.url ?? null }))}
        layout={view.layouts.some((option) => option.value === layout) ? layout : "sideBySide"}
        columns={view.columns}
      />
      <div className="grid gap-small" style={{ gridTemplateColumns: `repeat(${String(Math.max(view.columns, 1))}, minmax(0, 1fr))` }}>
        {tracks.map((track) => (
          <p key={track.id} className="m-0 truncate text-body font-semibold text-text">
            {track.title}
          </p>
        ))}
      </div>
    </div>
  );
}
