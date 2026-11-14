import { Check, RotateCw, Star } from "lucide-react";
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";

import { commands, type PresetsView_Serialize, type PresetTileItem_Serialize } from "../../bindings";
import { MediaTag, SectionLabel } from "../../design/Components";
import { cx } from "../../design/cx";
import { HoverHint } from "../../design/HoverHint";
import { LoopingVideo } from "../../design/LoopingVideo";
import { RemoteThumbnail } from "../../design/RemoteThumbnail";
import { Spinner } from "../../design/Spinner";
import { focusRing } from "../../design/Styles";
import { queryKeys } from "../../lib/queryKeys";
import { showError } from "./createData";

/**
 * Restyle's style gallery (`PresetSection.swift`): search, refresh, then Selected, Favorites, Recent and the whole
 * catalog. Picking a tile sets the style, or adds it to a batch or takes it out; Rust decides which.
 */
export function PresetSection({
  presets,
  onPick,
}: {
  presets: PresetsView_Serialize;
  onPick: (id: string) => void;
}) {
  const client = useQueryClient();
  const [query, setQuery] = useState(presets.query);

  const search = (text: string) => {
    setQuery(text);
    commands.createPresetQuery(text).then(
      (snapshot) => {
        client.setQueryData(queryKeys.create.snapshot, snapshot);
      },
      (error: unknown) => {
        showError("Can't search the styles", error);
      },
    );
  };
  const refresh = () => {
    commands.createRefreshPresets().catch((error: unknown) => {
      showError("Can't refresh the styles", error);
    });
  };
  const toggleFavorite = (id: string) => {
    commands.createToggleFavoritePreset(id).catch((error: unknown) => {
      showError("Can't change the favorites", error);
    });
  };

  return (
    <section className="flex flex-col gap-small">
      <div className="flex items-center">
        <div className="flex-1">
          <SectionLabel title={presets.title} detail={presets.detail ?? undefined} />
        </div>
        {/* A spinner while the list reloads: the catalog is usually unchanged, so nothing else moves. */}
        <HoverHint text="Refresh styles">
          <button
            type="button"
            aria-label="Refresh styles"
            disabled={presets.isRefreshing}
            className={cx(
              "flex size-24 items-center justify-center text-accent enabled:cursor-pointer disabled:text-faint",
              focusRing,
            )}
            onClick={refresh}
          >
            {presets.isRefreshing ? <Spinner /> : <RotateCw aria-hidden size={13} strokeWidth={2.25} />}
          </button>
        </HoverHint>
      </div>
      {presets.notice !== null && <p className="m-0 text-caption text-warning">{presets.notice}</p>}
      {presets.isLoaded ? (
        <>
          <input
            type="text"
            aria-label="Search styles"
            placeholder="Search styles"
            value={query}
            className="h-32 w-full rounded-field border border-stroke bg-well px-medium text-body text-text outline-none placeholder:text-faint focus:border-accent"
            onChange={(event) => {
              search(event.target.value);
            }}
          />
          {presets.failure !== null && <p className="m-0 text-caption text-faint">{presets.failure}</p>}
          {presets.sections.length === 0 && presets.isRefreshing && (
            <div className="flex justify-center">
              <Spinner />
            </div>
          )}
          {presets.sections.map((section) => (
            <div key={section.kind} className="flex flex-col gap-small">
              {presets.showsTitles && <SectionLabel title={section.title} />}
              <div className="grid grid-cols-2 gap-small">
                {section.items.map((item) => (
                  <PresetTile
                    key={`${section.kind}-${item.id}`}
                    item={item}
                    isSelected={presets.selection.includes(item.id)}
                    onPick={() => {
                      onPick(item.id);
                    }}
                    onToggleFavorite={() => {
                      toggleFavorite(item.id);
                    }}
                  />
                ))}
              </div>
            </div>
          ))}
        </>
      ) : (
        <div className="flex justify-center">
          <Spinner />
        </div>
      )}
    </section>
  );
}

function PresetTile({
  item,
  isSelected,
  onPick,
  onToggleFavorite,
}: {
  item: PresetTileItem_Serialize;
  isSelected: boolean;
  onPick: () => void;
  onToggleFavorite: () => void;
}) {
  const [isHovering, setIsHovering] = useState(false);
  // A selected style that left the catalog can still be taken out of a batch.
  const isDisabled = !item.isAvailable && !isSelected;
  const tile = (
    <button
      type="button"
      disabled={isDisabled}
      aria-pressed={isSelected}
      aria-label={item.isAvailable ? item.name : `${item.name}, unavailable`}
      className={cx(
        "relative h-76 w-full overflow-hidden rounded-field border bg-well enabled:cursor-pointer",
        isSelected ? "border-2 border-accent" : "border-stroke",
        !item.isAvailable && "opacity-45",
        focusRing,
      )}
      onClick={onPick}
      onMouseEnter={() => {
        setIsHovering(true);
      }}
      onMouseLeave={() => {
        setIsHovering(false);
      }}
    >
      <RemoteThumbnail url={item.previewUrl} className="absolute inset-0 size-full" />
      {isHovering && item.isAvailable && item.previewVideoUrl !== null && (
        <span className="absolute inset-0">
          <LoopingVideo src={item.previewVideoUrl} />
        </span>
      )}
      <span className="absolute bottom-6 left-6 flex flex-col items-start gap-3">
        {!item.isAvailable && <MediaTag text="Unavailable" />}
        <MediaTag text={item.name} />
      </span>
      {isSelected && (
        <span className="absolute top-6 right-6 flex size-18 items-center justify-center rounded-full bg-accent text-ink">
          <Check aria-hidden size={10} strokeWidth={3.5} />
        </span>
      )}
    </button>
  );
  const favoriteTitle = item.isFavorite ? "Remove from favorites" : "Add to favorites";
  return (
    <div className="relative">
      {item.isAvailable ? tile : <HoverHint text="No longer available">{tile}</HoverHint>}
      {/* Outside the tile's button so an unavailable favorite can still be unstarred. */}
      {(item.isAvailable || item.isFavorite) && (
        <HoverHint text={favoriteTitle} className="absolute top-3 left-3">
          <button
            type="button"
            aria-label={favoriteTitle}
            className={cx(
              "flex size-18 cursor-pointer items-center justify-center rounded-full bg-scrim text-accent",
              focusRing,
            )}
            onClick={onToggleFavorite}
          >
            <Star aria-hidden size={9} strokeWidth={3} fill={item.isFavorite ? "currentColor" : "none"} />
          </button>
        </HoverHint>
      )}
    </div>
  );
}
