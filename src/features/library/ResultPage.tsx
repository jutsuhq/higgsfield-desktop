import { useQuery } from "@tanstack/react-query";
import { useNavigate, useParams } from "@tanstack/react-router";
import {
  ArrowDownCircle,
  ArrowRight,
  Check,
  ChevronLeft,
  ChevronRight,
  CircleX,
  Combine,
  Copy,
  Download,
  Ellipsis,
  Folder,
  HardDrive,
  RefreshCw,
  Star,
} from "lucide-react";
import { useEffect, useRef, useState, type MouseEvent } from "react";

import { commands, type CullingAction, type InputView_Serialize, type ResultDetail_Serialize } from "../../bindings";
import { EmptyState, PageHeader, SectionLabel } from "../../design/Components";
import { cx } from "../../design/cx";
import { Spinner } from "../../design/Spinner";
import { Button, focusRing } from "../../design/Styles";
import { isEditingText } from "../../design/textFocus";
import { queryKeys } from "../../lib/queryKeys";
import { useShortcut } from "../../lib/shortcuts/useShortcut";
import { loadJob, openFollowUp } from "../create/createData";
import { ComparePlayer } from "../compare/ComparePlayer";
import { CullingButtons } from "./CullingButtons";
import { EditingExportButton } from "./EditingExport";
import { EnhanceMenu, MadeFromLink } from "./Enhance";
import { RenderFinalSection, copyVideos } from "./followUps";
import { ContextMenu, DeleteDialog, ImageDetailsDialog, type MenuItem } from "./LibraryDialogs";
import { AgentShownOnDeviceSheet, OnDeviceRunStatus } from "./OnDeviceSheet";
import { useDuplicate, useSaveTemplate } from "./ResultMoreActions";
import { AgentShownSubtitlesSheet, SubtitleEditor, SubtitlesSection } from "./Subtitles";
import { cull, useLibraryView, useResultDetail } from "./libraryData";

/**
 * A result's page (`LibraryDetail.swift`): a top bar with the pager, the verdict, the star, the file's actions, Export
 * for Editing and More; then the player on its stage beside the details, or above them in a narrow window. Esc goes
 * back to the Library, ← and → step through the grid's results, P, X and U keep, reject and clear it.
 */
export function ResultPage() {
  const id = useParams({ strict: false }).id ?? "";
  const navigate = useNavigate();
  const query = useResultDetail(id);
  const detail = query.data;
  const order = useLibraryView((state) => state.order);
  const showsRejected = useLibraryView((state) => state.filter.showsRejected);
  const [image, setImage] = useState<{ id: string; versionId: string | null } | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);
  const [isMenuOpen, setIsMenuOpen] = useState(false);
  // Edit Subtitles replaces the page with the editor until Done or Cancel; another result closes it.
  const [editing, setEditing] = useState<string | null>(null);
  const resultId = detail?.id ?? id;
  const isEditingSubtitles = editing === resultId;
  const isModal = image !== null || isDeleting || isMenuOpen || isEditingSubtitles;
  const position = order.indexOf(resultId);

  const back = () => void navigate({ to: "/library" });
  const go = (offset: number) => {
    const next = order[position + offset];
    if (position >= 0 && next !== undefined) void navigate({ to: "/library/result/$id", params: { id: next } });
  };
  // A verdict steps on to the next result, so a series is culled from here too; taking one back stays
  // (`LibraryView.cullShown`).
  const applyCulling = (action: CullingAction) => {
    void cull(action, [resultId], { id: resultId, order, showsRejected }).then((outcome) => {
      const next = outcome.cursor ?? null;
      if (outcome.action !== null && outcome.action !== "clear" && next !== null && next !== resultId) {
        void navigate({ to: "/library/result/$id", params: { id: next } });
      }
    });
  };

  useEffect(() => {
    if (isModal) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || isEditingText() || event.ctrlKey || event.altKey || event.metaKey) return;
      if (event.key === "Escape") back();
      else if (event.key === "ArrowLeft") go(-1);
      else if (event.key === "ArrowRight") go(1);
      else return;
      event.preventDefault();
    };
    const onUndo = (event: KeyboardEvent) => {
      if (event.ctrlKey && !event.shiftKey && event.key.toLowerCase() === "z" && !isEditingText()) {
        event.preventDefault();
        void commands.libraryUndoCulling();
      }
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("keydown", onUndo);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keydown", onUndo);
    };
  });

  useShortcut("keep", () => { applyCulling("keep"); }, !isModal && detail !== undefined);
  useShortcut("reject", () => { applyCulling("reject"); }, !isModal && detail !== undefined);
  useShortcut("clearCulling", () => { applyCulling("clear"); }, !isModal && detail !== undefined);
  useShortcut(
    "star",
    () => {
      if (detail?.canStar) void commands.librarySetStarred([detail.id], !detail.isStarred);
    },
    !isModal && detail?.canStar === true,
    detail?.starTitle ?? null,
  );
  useShortcut(
    "deleteResult",
    () => {
      if (detail && !detail.needsImport) setIsDeleting(true);
    },
    !isModal && detail?.needsImport === false,
  );

  if (query.isError) {
    return (
      <div className="flex h-full flex-col px-page pt-page-top">
        <PageHeader title="Library" />
        <EmptyState
          icon={CircleX}
          title="This result is no longer in the library."
          action={<Button variant="secondary" icon={ChevronLeft} title="Library" onClick={back} />}
        />
      </div>
    );
  }
  if (!detail) {
    return (
      <div className="flex h-full flex-col px-page pt-page-top">
        <PageHeader title="Library" />
        <div className="flex flex-1 items-center justify-center">
          <Spinner />
        </div>
      </div>
    );
  }
  const aspect = detail.aspectRatio ?? 16 / 9;

  if (isEditingSubtitles) {
    return (
      <SubtitleEditor
        key={detail.id}
        id={detail.id}
        videoUrl={detail.result.url ?? detail.remoteUrl}
        aspect={aspect}
        onClose={() => {
          setEditing(null);
        }}
      />
    );
  }

  return (
    <div className="flex h-full flex-col gap-large px-page pt-page-top pb-page">
      <TopBar
        detail={detail}
        position={position}
        count={order.length}
        onBack={back}
        onStep={go}
        onCull={applyCulling}
        onMenu={setIsMenuOpen}
        onDelete={() => {
          setIsDeleting(true);
        }}
      />
      <div className="@container min-h-0 flex-1">
        <div
          className={cx(
            "flex h-full flex-col gap-x-large overflow-y-auto [scrollbar-width:none]",
            aspect < 1
              ? "@min-[600px]:flex-row @min-[600px]:overflow-hidden"
              : "@min-[780px]:flex-row @min-[780px]:overflow-hidden",
          )}
        >
          <ResultStage
            detail={detail}
            aspect={aspect}
            isActive={!isModal}
            className={cx(
              "h-380 shrink-0",
              aspect < 1 ? "@min-[600px]:h-full @min-[600px]:min-w-0 @min-[600px]:flex-1" : "@min-[780px]:h-full @min-[780px]:min-w-0 @min-[780px]:flex-1",
            )}
          />
          <div
            className={cx(
              "shrink-0 [scrollbar-width:none]",
              aspect < 1 ? "@min-[600px]:w-320 @min-[600px]:overflow-y-auto" : "@min-[780px]:w-320 @min-[780px]:overflow-y-auto",
            )}
          >
            <Details
              detail={detail}
              onMenu={setIsMenuOpen}
              onOpenImage={(materialId, versionId) => {
                setImage({ id: materialId, versionId });
              }}
              onEditSubtitles={() => {
                setEditing(detail.id);
              }}
            />
          </div>
        </div>
      </div>
      {isDeleting && (
        <DeleteDialog
          ids={[detail.id]}
          onDone={(deleted) => {
            setIsDeleting(false);
            if (deleted) back();
          }}
        />
      )}
      {image && (
        <ImageDetailsDialog
          id={image.id}
          versionId={image.versionId}
          onClose={() => {
            setImage(null);
          }}
        />
      )}
    </div>
  );
}

/** Library, the pager, Keep · Reject · Clear, the star, Show in File Manager and Copy Video, Export for Editing, More. */
function TopBar({
  detail,
  position,
  count,
  onBack,
  onStep,
  onCull,
  onMenu,
  onDelete,
}: {
  detail: ResultDetail_Serialize;
  position: number;
  count: number;
  onBack: () => void;
  onStep: (offset: number) => void;
  onCull: (action: CullingAction) => void;
  onMenu: (isOpen: boolean) => void;
  onDelete: () => void;
}) {
  const hasFile = detail.result.exists;
  return (
    <div className="flex items-center gap-small">
      <Button variant="secondary" icon={ChevronLeft} title="Library" hint="Back to Library (Esc)" onClick={onBack} />
      <span className="flex-1" />
      {count > 1 && position >= 0 && (
        <div className="mr-medium flex items-center gap-4">
          <Button
            variant="icon"
            icon={ChevronLeft}
            title="Previous"
            hint="Previous result (←)"
            disabled={position === 0}
            onClick={() => {
              onStep(-1);
            }}
          />
          <span className="min-w-52 text-center text-small font-semibold text-faint tabular-nums">
            {position + 1} of {count}
          </span>
          <Button
            variant="icon"
            icon={ChevronRight}
            title="Next"
            hint="Next result (→)"
            disabled={position === count - 1}
            onClick={() => {
              onStep(1);
            }}
          />
        </div>
      )}
      {detail.needsImport && (
        // Port-only: a result only on Higgsfield opens here too, and comes to the Library from this button.
        <Button
          variant="icon"
          icon={ArrowDownCircle}
          title="Download to Library"
          hint="Download to Library"
          onClick={() => void commands.libraryDownload(detail.id)}
        />
      )}
      <div className="flex items-center gap-6">
        <CullingButtons count={1} current={detail.culling} onCull={onCull} />
      </div>
      {detail.canStar && (
        <Button
          variant="icon"
          icon={Star}
          isIconFilled={detail.isStarred}
          title={detail.isStarred ? "Unstar" : "Star"}
          hint={detail.isStarred ? "Unstar" : "Star"}
          aria-pressed={detail.isStarred}
          className={cx(detail.isStarred && "text-accent")}
          onClick={() => void commands.librarySetStarred([detail.id], !detail.isStarred)}
        />
      )}
      <div className="flex items-center gap-6">
        <Button
          variant="icon"
          icon={Folder}
          title="Show in File Manager"
          hint="Show in File Manager"
          disabled={!hasFile}
          onClick={() => void commands.libraryShowInFolder([detail.id])}
        />
        <Button
          variant="icon"
          icon={Copy}
          title="Copy Video"
          hint="Copy video"
          disabled={!hasFile}
          onClick={() => void copyVideos([detail.id])}
        />
      </div>
      <EditingExportButton id={detail.id} />
      <MoreMenu detail={detail} onOpen={onMenu} onDelete={onDelete} />
    </div>
  );
}

/**
 * More (`LibraryDetail.moreMenu`): Duplicate, Edit… and Extend…, Share…, Copy Prompt, Copy Request ID, Compare with…,
 * Save as Template…, then Delete from Library….
 */
function MoreMenu({
  detail,
  onOpen,
  onDelete,
}: {
  detail: ResultDetail_Serialize;
  onOpen: (isOpen: boolean) => void;
  onDelete: () => void;
}) {
  const navigate = useNavigate();
  const button = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<{ x: number; y: number } | null>(null);
  const duplicate = useDuplicate(detail.id);
  const template = useSaveTemplate(detail.id);
  const hasFile = detail.result.exists;
  const candidates =
    useQuery({
      queryKey: [...queryKeys.library.all, "candidates", detail.id],
      queryFn: () => commands.libraryCompareCandidates(detail.id),
      enabled: hasFile && detail.menu.canCompare,
    }).data ?? [];
  const followUp = (mode: "edit" | "extend") => () => {
    void openFollowUp(detail.id, mode).then(() => navigate({ to: "/create" }));
  };
  const copy = (text: string) => () => void navigator.clipboard.writeText(text);

  const items: (MenuItem | "divider")[] = [
    { title: "Duplicate", isDisabled: duplicate.isDisabled, run: duplicate.run },
    ...(detail.menu.canEditVideo
      ? [
          { title: "Edit…", hint: "Change this result as a prompt says, with Seedance 2.5", run: followUp("edit") },
          { title: "Extend…", hint: "Continue this result past its last frame, with Seedance 2.5", run: followUp("extend") },
        ]
      : []),
    ...(hasFile
      ? [
          {
            title: "Share…",
            hint: "Show in the file manager to share from there",
            run: () => void commands.libraryShowInFolder([detail.id]),
          },
        ]
      : []),
    ...(detail.prompt === "" ? [] : [{ title: "Copy Prompt", run: copy(detail.prompt) }]),
    ...(detail.requestId === null ? [] : [{ title: "Copy Request ID", run: copy(detail.requestId) }]),
    ...(hasFile && candidates.length > 0
      ? [
          {
            title: "Compare with…",
            run: () => undefined,
            children: candidates.map((candidate) => ({
              title: candidate.title,
              run: () =>
                void navigate({ to: "/library/compare/$ids", params: { ids: `${detail.id},${candidate.id}` } }),
            })),
          },
        ]
      : []),
    ...(detail.needsImport ? [] : [{ title: "Save as Template…", run: template.run }]),
    "divider",
    { title: "Delete from Library…", isDestructive: true, isDisabled: detail.needsImport, run: onDelete },
  ];

  return (
    <>
      <div ref={button}>
        <Button
          variant="icon"
          icon={Ellipsis}
          title="More"
          hint="More"
          aria-haspopup="menu"
          onClick={() => {
            const rect = button.current?.getBoundingClientRect();
            setPosition({ x: rect?.left ?? 0, y: rect?.bottom ?? 0 });
            onOpen(true);
          }}
        />
      </div>
      {position && (
        <ContextMenu
          x={position.x}
          y={position.y}
          items={items}
          onClose={() => {
            setPosition(null);
            onOpen(false);
          }}
        />
      )}
      {duplicate.dialog}
      {template.dialog}
    </>
  );
}

/**
 * The player on a dark stage (`ResultStage`): before/after with a divider when both files are here, otherwise the one
 * that is.
 */
function ResultStage({
  detail,
  aspect,
  isActive,
  className,
}: {
  detail: ResultDetail_Serialize;
  aspect: number;
  isActive: boolean;
  className: string;
}) {
  const result = detail.result.url ?? detail.remoteUrl;
  const source = detail.source?.file.exists === true ? detail.source : null;
  const sourceUrl = source?.file.url ?? null;
  return (
    <div
      className={cx(
        "flex items-center justify-center overflow-hidden rounded-card border border-stroke bg-well p-medium",
        className,
      )}
    >
      {result !== null && source !== null && sourceUrl !== null ? (
        <div className="max-h-full w-full">
          <ComparePlayer
            key={detail.id}
            layout="divider"
            columns={2}
            aspect={aspect}
            isActive={isActive}
            tracks={[
              {
                id: `${detail.id}-source`,
                labels: ["Before"],
                url: sourceUrl,
                source: { start: source.start ?? 0, end: source.end ?? Number.POSITIVE_INFINITY },
              },
              { id: `${detail.id}-result`, labels: ["After"], url: result },
            ]}
          />
        </div>
      ) : result !== null ? (
        <video
          key={detail.id}
          src={result}
          controls
          playsInline
          className="max-h-full max-w-full rounded-small bg-black"
          style={{ aspectRatio: aspect }}
        />
      ) : source !== null && sourceUrl !== null ? (
        // A source clip starts at its range and stops at its end (`LibraryPlayer`).
        <video
          key={detail.id}
          src={source.start === null ? sourceUrl : `${sourceUrl}#t=${String(source.start)}`}
          controls
          playsInline
          className="max-h-full max-w-full rounded-small bg-black"
          style={{ aspectRatio: aspect }}
          onTimeUpdate={(event) => {
            const video = event.currentTarget;
            if (source.end !== null && video.currentTime >= source.end) video.pause();
          }}
        />
      ) : (
        <EmptyState icon={CircleX} title="File not found" />
      )}
    </div>
  );
}

function Details({
  detail,
  onOpenImage,
  onEditSubtitles,
  onMenu,
}: {
  detail: ResultDetail_Serialize;
  onOpenImage: (materialId: string, versionId: string) => void;
  onEditSubtitles: () => void;
  onMenu: (isOpen: boolean) => void;
}) {
  const navigate = useNavigate();
  const { menu } = detail;
  const canRenderFinal = menu.followUp === "finalize" || menu.followUp === "renderFinal";
  const hasFile = detail.result.exists;
  const [isJoining, setIsJoining] = useState(false);
  return (
    <div className="flex flex-col gap-x-large">
      <Summary detail={detail} onMenu={onMenu} />
      {detail.chain.length > 1 && <ChainSteps detail={detail} />}
      {detail.operationSource !== null && <MadeFromLink link={detail.operationSource} />}
      <MadeFromImages detail={detail} onOpenImage={onOpenImage} />
      <div className="flex flex-col gap-medium">
        {canRenderFinal && <RenderFinalSection id={detail.id} />}
        {menu.canRerun && (
          <Button
            variant={canRenderFinal ? "secondary" : "primary"}
            icon={RefreshCw}
            title="Edit & Re-run"
            className="w-full"
            onClick={() => void loadJob(detail.id).then(() => navigate({ to: "/create" }))}
          />
        )}
        {hasFile && detail.enhance !== null && <EnhanceMenu id={detail.id} menu={detail.enhance} />}
        <OnDeviceRunStatus id={detail.id} />
        <AgentShownOnDeviceSheet id={detail.id} />
        {hasFile && <SubtitlesSection id={detail.id} title={detail.title} onEdit={onEditSubtitles} />}
        {hasFile && <AgentShownSubtitlesSheet id={detail.id} title={detail.title} />}
        {hasFile && menu.canJoin && (
          <Button
            variant="secondary"
            icon={Combine}
            title="Join with Original…"
            hint="Join into one video"
            className="w-full"
            disabled={isJoining}
            onClick={() => {
              setIsJoining(true);
              void commands.libraryJoinWithOriginal(detail.id).finally(() => {
                setIsJoining(false);
              });
            }}
          />
        )}
      </div>
      <div className="flex flex-col gap-small font-mono text-caption-plus font-medium text-faint tabular-nums select-text">
        <span>{detail.billing}</span>
        {detail.timing !== null && <span title="Queue wait: until generation started">{detail.timing}</span>}
      </div>
    </div>
  );
}

/** The headline, the title and the settings, the prompt, and the inputs that went in (`ResultSummary`). */
function Summary({ detail, onMenu }: { detail: ResultDetail_Serialize; onMenu: (isOpen: boolean) => void }) {
  const [isSavingInputs, setIsSavingInputs] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const source = detail.source;
  const references = detail.referencePreviews;
  const frameWidth = 64 * Math.min(Math.max(detail.aspectRatio ?? 16 / 9, 0.56), 1.78);
  /** Save Inputs…, or one input with `index`: one file asks where with its name, several pick a folder. */
  const save = (index: number | null) => {
    if (isSavingInputs) return;
    setIsSavingInputs(true);
    void commands.librarySaveInputs(detail.id, index).finally(() => {
      setIsSavingInputs(false);
    });
  };
  // A tile's menu: Save Source video… and, while the file is here, Show in File Manager (`ResultSummary.inputMenu`).
  const openMenu = (event: MouseEvent, matches: (input: InputView_Serialize) => boolean) => {
    event.preventDefault();
    const index = detail.inputs.findIndex(matches);
    const input = detail.inputs[index];
    if (input === undefined) return;
    const items: MenuItem[] = [
      { title: `Save ${input.title}…`, run: () => { save(index); } },
      ...(input.isHere
        ? [{ title: "Show in File Manager", run: () => void commands.libraryRevealInput(detail.id, index) }]
        : []),
    ];
    setMenu({ x: event.clientX, y: event.clientY, items });
    onMenu(true);
  };
  return (
    <div className="flex flex-col gap-x-large">
      <div className="flex flex-col gap-small">
        <span className="text-small font-medium text-faint">{detail.headline}</span>
        <h1 className="m-0 line-clamp-2 text-[20px] font-bold text-text">{detail.title}</h1>
        <ul className="m-0 flex list-none flex-wrap gap-6 p-0 pt-2">
          {detail.facts.map((fact) => (
            <li
              key={fact}
              className="flex h-24 items-center rounded-full bg-glass px-10 text-caption-plus font-semibold text-muted"
            >
              {fact}
            </li>
          ))}
        </ul>
      </div>
      {detail.prompt !== "" && (
        <section className="flex flex-col gap-small rounded-field border border-stroke bg-well p-14">
          <div className="flex items-center gap-small">
            <SectionLabel title="Prompt" />
            <CopyButton text={detail.prompt} />
          </div>
          <p className="m-0 text-body leading-[1.5] whitespace-pre-wrap text-text select-text">{detail.prompt}</p>
        </section>
      )}
      <section className="flex flex-col gap-small">
        <div className="flex items-center gap-small">
          <SectionLabel title="Inputs" detail={detail.rangeText ?? undefined} />
          {isSavingInputs ? (
            <Spinner label="Saving inputs" />
          ) : (
            detail.inputs.length > 0 && (
              <Button
                variant="icon"
                icon={Download}
                title="Save Inputs…"
                hint="Save inputs"
                onClick={() => {
                  save(null);
                }}
              />
            )
          )}
        </div>
        {(source !== null || references.length > 0) && (
          <div className="flex items-center gap-small">
            {source !== null && (
              <span
                className="relative h-64 shrink-0 overflow-hidden rounded-small border border-stroke bg-well"
                style={{ width: frameWidth }}
                onContextMenu={(event) => {
                  openMenu(event, (input) => input.isSource);
                }}
              >
                {source.file.url !== null && source.file.exists ? (
                  <video
                    src={`${source.file.url}#t=${String(source.start ?? 0)}`}
                    muted
                    preload="metadata"
                    className="size-full object-cover"
                  />
                ) : (
                  <HardDrive aria-hidden size={16} className="absolute inset-0 m-auto text-faint" />
                )}
              </span>
            )}
            {references.slice(0, 4).map((url, index) => (
              <span
                // The order is the job's: a reference is its position.
                key={`${String(index)}-${url ?? ""}`}
                className="size-52 shrink-0 overflow-hidden rounded-small border border-stroke bg-well"
                onContextMenu={(event) => {
                  openMenu(event, (input) => input.reference === index + 1);
                }}
              >
                {url !== null && <img src={url} alt={`Reference ${String(index + 1)}`} className="size-full object-cover" />}
              </span>
            ))}
            {references.length > 4 && (
              <span className="text-small font-semibold text-muted">+{references.length - 4}</span>
            )}
          </div>
        )}
        <span className="truncate text-caption-plus font-medium text-faint">{detail.sourceLine}</span>
      </section>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menu.items}
          onClose={() => {
            setMenu(null);
            onMenu(false);
          }}
        />
      )}
    </div>
  );
}

/** Copy, then Copied with a check for a moment (`CopyButton`). */
function CopyButton({ text }: { text: string }) {
  const [didCopy, setDidCopy] = useState(false);
  useEffect(() => {
    if (!didCopy) return;
    const timer = setTimeout(() => {
      setDidCopy(false);
    }, 1500);
    return () => {
      clearTimeout(timer);
    };
  }, [didCopy]);
  const Icon = didCopy ? Check : Copy;
  return (
    <button
      type="button"
      className={cx(
        "flex cursor-pointer items-center gap-4 text-caption font-semibold",
        didCopy ? "text-accent" : "text-muted",
        focusRing,
      )}
      onClick={() => {
        void navigator.clipboard.writeText(text);
        setDidCopy(true);
      }}
    >
      <Icon aria-hidden size={11} strokeWidth={2.5} />
      {didCopy ? "Copied" : "Copy"}
    </button>
  );
}

/** "draft 480p → final 720p → final 1080p"; every step but the current and a deleted one opens it (`ChainSteps`). */
function ChainSteps({ detail }: { detail: ResultDetail_Serialize }) {
  const navigate = useNavigate();
  return (
    <section className="flex flex-col gap-small">
      <SectionLabel title="Chain" />
      <div className="flex flex-wrap items-center gap-6">
        {detail.chain.map((step, index) => (
          <span key={`${step.id}-${String(index)}`} className="flex items-center gap-6">
            {index > 0 && <ArrowRight aria-hidden size={9} strokeWidth={3} className="text-faint" />}
            <button
              type="button"
              disabled={step.isDeleted || step.isCurrent}
              aria-current={step.isCurrent || undefined}
              title={step.isDeleted ? "Deleted from the library" : "Open this result"}
              onClick={() => void navigate({ to: "/library/result/$id", params: { id: step.id } })}
              className={cx(
                "flex h-24 cursor-pointer items-center rounded-full px-10 text-caption-plus font-semibold disabled:cursor-default",
                step.isCurrent ? "bg-accent text-ink" : "bg-glass",
                !step.isCurrent && (step.isDeleted ? "text-faint line-through" : "text-text"),
                focusRing,
              )}
            >
              {step.title}
            </button>
          </span>
        ))}
      </div>
    </section>
  );
}

/** The images the video was made from, each with its role; one that is gone stays, marked (`MadeFromImages`). */
function MadeFromImages({
  detail,
  onOpenImage,
}: {
  detail: ResultDetail_Serialize;
  onOpenImage: (materialId: string, versionId: string) => void;
}) {
  const navigate = useNavigate();
  if (detail.madeFrom.length === 0) return null;
  return (
    <section className="flex flex-col gap-small">
      <SectionLabel title="Made from" />
      <div className="flex flex-wrap gap-small">
        {detail.madeFrom.map((link) => {
          const isGone = link.availability === "unavailable";
          return (
            <button
              key={`${link.reference.materialID}-${link.reference.versionID}-${link.roleTitle}`}
              type="button"
              title={link.prompt === "" ? "Show this image" : link.prompt}
              onClick={() => {
                if (link.isVideo) void navigate({ to: "/library/result/$id", params: { id: link.reference.materialID } });
                else onOpenImage(link.reference.materialID, link.reference.versionID);
              }}
              className={cx("flex w-72 cursor-pointer flex-col gap-4 text-left", focusRing)}
            >
              <span
                className={cx(
                  "relative flex size-72 items-center justify-center overflow-hidden rounded-small border border-stroke bg-well",
                  isGone && "opacity-60",
                )}
              >
                {isGone || link.previewUrl === null ? (
                  <HardDrive aria-hidden size={16} className="text-faint" />
                ) : link.isVideo ? (
                  <video src={link.previewUrl} muted preload="metadata" className="size-full object-cover" />
                ) : (
                  <img src={link.previewUrl} alt="" className="size-full object-cover" />
                )}
              </span>
              <span className="text-caption font-semibold text-text">{link.roleTitle}</span>
              {link.note !== null && <span className="text-label text-faint">{link.note}</span>}
            </button>
          );
        })}
      </div>
    </section>
  );
}
