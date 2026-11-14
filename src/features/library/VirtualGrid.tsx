import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";

/**
 * The narrowest a tile gets before a column is dropped and the gap between tiles (`LibraryViewGrid.cardWidth`,
 * `cardSpacing`), then the text under the 16:9 frame: the 2 px gap and the two caption lines.
 */
const minTileWidth = 230;
const gap = 18;
const captionHeight = 38;
/** Rows drawn beyond the visible ones, so a fast scroll doesn't show blanks. */
const overscan = 2;
/** The size assumed before the first measurement, and where nothing can be measured (tests). */
const fallback = { width: 1024, height: 800 };

/**
 * A grid of equal tiles that only mounts the rows near the visible part, so thousands of cards scroll smoothly. Rows
 * have a fixed height from the tile width; `onColumns` reports the column count for the arrow keys, and `scrollTo`
 * brings the current card into view.
 */
export function VirtualGrid<Item>({
  items,
  itemKey,
  renderItem,
  onColumns,
  scrollTo,
  footer,
}: {
  items: readonly Item[];
  itemKey: (item: Item) => string;
  renderItem: (item: Item) => ReactNode;
  onColumns: (columns: number) => void;
  scrollTo: string | null;
  footer?: ReactNode;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState(fallback);
  const [scrollTop, setScrollTop] = useState(0);

  useLayoutEffect(() => {
    const element = scroller.current;
    if (!element) return;
    const measure = () => {
      const width = element.clientWidth || fallback.width;
      const height = element.clientHeight || fallback.height;
      setSize((current) => (current.width === width && current.height === height ? current : { width, height }));
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, []);

  const columns = Math.max(1, Math.floor((size.width + gap) / (minTileWidth + gap)));
  const tileWidth = (size.width - gap * (columns - 1)) / columns;
  const rowHeight = Math.ceil((tileWidth * 9) / 16 + captionHeight + gap);
  const rowCount = Math.ceil(items.length / columns);
  const first = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const last = Math.min(rowCount, Math.ceil((scrollTop + size.height) / rowHeight) + overscan);

  useEffect(() => {
    onColumns(columns);
  }, [columns, onColumns]);

  useEffect(() => {
    const element = scroller.current;
    if (scrollTo === null || !element) return;
    const index = items.findIndex((item) => itemKey(item) === scrollTo);
    if (index < 0) return;
    const top = Math.floor(index / columns) * rowHeight;
    if (top < element.scrollTop) element.scrollTop = top;
    else if (top + rowHeight > element.scrollTop + size.height) element.scrollTop = top + rowHeight - size.height;
    // Only a new current card scrolls: the user's own scrolling leaves it out of view.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scrollTo]);

  const rows: ReactNode[] = [];
  for (let row = first; row < last; row += 1) {
    const slice = items.slice(row * columns, (row + 1) * columns);
    rows.push(
      <div
        key={row}
        role="row"
        className="absolute right-0 left-0 grid"
        style={{
          top: row * rowHeight,
          height: rowHeight - gap,
          gap,
          gridTemplateColumns: `repeat(${String(columns)}, minmax(0, 1fr))`,
        }}
      >
        {slice.map((item) => (
          <div key={itemKey(item)} className="min-w-0">
            {renderItem(item)}
          </div>
        ))}
      </div>,
    );
  }

  return (
    <div
      ref={scroller}
      className="min-h-0 flex-1 overflow-y-auto pb-page"
      onScroll={(event) => {
        setScrollTop(event.currentTarget.scrollTop);
      }}
    >
      <div role="grid" aria-rowcount={rowCount} className="relative" style={{ height: rowCount * rowHeight }}>
        {rows}
      </div>
      {footer}
    </div>
  );
}
