/**
 * The clock of Compare (`CompareClock`, `CompareTimeline`, `CompareDivider` in `BeforeAfter.swift` and
 * `Compare.swift`): playback timing that only the window can know, kept out of the components so it is tested.
 */

/** A video further than this from the clock is moved back onto it. */
export const driftTolerance = 0.1;
/** One arrow-key press moves the divider by this fraction of the width. */
export const dividerStep = 0.05;

/** Every video plays to the shortest one's end: the clock stops there. `0` while no length is known. */
export function compareDuration(durations: readonly number[]): number {
  const known = durations.filter((duration) => Number.isFinite(duration) && duration > 0);
  return known.length === 0 ? 0 : Math.min(...known);
}

/** Whether a video at `videoTime` has drifted from the clock at `clockTime`. */
export function hasDrifted(videoTime: number, clockTime: number): boolean {
  return Math.abs(videoTime - clockTime) > driftTolerance;
}

/** Where a source plays at clock time `time`: from the start of the range that was sent, never past its end. */
export function sourceTime(start: number, end: number, time: number): number {
  return Math.min(start + Math.max(time, 0), end);
}

/** The divider after a key: ← and → move it a step, kept within 0…1; `null` for any other key. */
export function dividerAfterKey(position: number, key: string): number | null {
  const step = key === "ArrowLeft" ? -dividerStep : key === "ArrowRight" ? dividerStep : null;
  return step === null ? null : clamped(position + step);
}

/** The divider following a pointer at `x` within a player `width` wide. */
export function dividerAt(x: number, width: number): number {
  return width > 0 ? clamped(x / width) : 0.5;
}

/** "0:04.2". */
export function timecode(seconds: number): string {
  const safe = Number.isFinite(seconds) && seconds > 0 ? seconds : 0;
  const minutes = Math.floor(safe / 60);
  const rest = safe - minutes * 60;
  return `${String(minutes)}:${rest.toFixed(1).padStart(4, "0")}`;
}

function clamped(position: number): number {
  return Math.min(Math.max(position, 0), 1);
}
