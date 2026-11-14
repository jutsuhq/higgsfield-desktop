import { ChevronsLeftRight } from "lucide-react";

/**
 * The compare divider (`DividerLine` in ComparePlayer.swift): a 2-wide line in the signature gradient with a fuchsia
 * glow, and a neutral grip in its middle. The gradient marks the effect, not a control. It fills the height of its
 * positioned parent and is centered on that parent's middle; it takes no pointer events.
 */
export function DividerLine() {
  return (
    <span aria-hidden className="pointer-events-none absolute inset-y-0 left-1/2 flex items-center">
      <span className="absolute inset-y-0 left-0 w-2 -translate-x-1/2 bg-(image:--signature-vertical) shadow-signature-glow" />
      <span className="relative flex size-24 -translate-x-1/2 items-center justify-center rounded-full border-[0.5px] border-glass-stroke bg-scrim text-text">
        <ChevronsLeftRight size={11} strokeWidth={3} />
      </span>
    </span>
  );
}
