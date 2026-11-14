/**
 * The grid's view state (`LibrarySelection`, `CullingTargets`, `LibraryCursor`): which cards are picked for bulk
 * actions, what a culling key acts on, and the card the arrow keys move between.
 */

export interface Selection {
  /** The selected results. */
  ids: ReadonlySet<string>;
  /** The card a Shift-click extends from: the last one toggled on. */
  anchor: string | null;
}

export const emptySelection: Selection = { ids: new Set(), anchor: null };

/** Ctrl-click: picks the card, or puts it back. */
export function toggled(selection: Selection, id: string): Selection {
  const ids = new Set(selection.ids);
  if (ids.has(id)) {
    ids.delete(id);
    return { ids, anchor: selection.anchor === id ? null : selection.anchor };
  }
  ids.add(id);
  return { ids, anchor: id };
}

/** Shift-click: adds every card from the anchor to `id`, in grid order; without an anchor it toggles. */
export function extended(selection: Selection, id: string, order: readonly string[]): Selection {
  const start = selection.anchor === null ? -1 : order.indexOf(selection.anchor);
  const end = order.indexOf(id);
  if (start < 0 || end < 0) return toggled(selection, id);
  const ids = new Set(selection.ids);
  for (const member of order.slice(Math.min(start, end), Math.max(start, end) + 1)) ids.add(member);
  return { ids, anchor: selection.anchor };
}

/** Ctrl+A: every card shown. */
export function selectedAll(order: readonly string[]): Selection {
  return { ids: new Set(order), anchor: order[0] ?? null };
}

/** The selected cards among those shown, in grid order: a hidden one is never acted on. */
export function chosen(selection: Selection, visible: readonly string[]): string[] {
  return visible.filter((id) => selection.ids.has(id));
}

/** What a culling key or a bulk button acts on: the visible selected cards, else the current one. */
export function cullingTargets(visible: readonly string[], selection: Selection, current: string | null): string[] {
  const picked = chosen(selection, visible);
  if (picked.length > 0) return picked;
  return current !== null && visible.includes(current) ? [current] : [];
}

export type CursorMove = "left" | "right" | "up" | "down";

/** The card after the move; the first card when none is current yet or the current one is gone. */
export function movedCursor(
  current: string | null,
  move: CursorMove,
  ids: readonly string[],
  columns: number,
): string | null {
  const index = current === null ? -1 : ids.indexOf(current);
  if (index < 0) return ids[0] ?? null;
  const row = Math.max(columns, 1);
  const step = { left: -1, right: 1, up: -row, down: row }[move];
  return ids[Math.min(Math.max(index + step, 0), ids.length - 1)] ?? null;
}

/** "12 selected", the scope a bulk button names. */
export function selectionTitle(count: number): string {
  return `${count.toLocaleString()} selected`;
}

/** "Keep", "Reject 3 results", "Clear verdict", "Clear 3 verdicts". */
export function cullingHint(action: "keep" | "reject" | "clear", count: number): string {
  const title = { keep: "Keep", reject: "Reject", clear: "Clear verdict" }[action];
  if (count <= 1) return title;
  return action === "clear" ? `Clear ${String(count)} verdicts` : `${title} ${String(count)} results`;
}
