import {
  cloneElement,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactElement,
} from "react";
import { createPortal } from "react-dom";

import { cx } from "./cx";

/** How long the pointer rests on a control before its hint shows (HoverHint.swift). */
export const hintDelayMs = 400;

/** The one hint on screen: showing another hides it. */
let hideShownHint: (() => void) | undefined;

function cssLength(name: string): number {
  return Number.parseFloat(getComputedStyle(document.documentElement).getPropertyValue(name)) || 0;
}

interface HoverHintProps {
  /** An empty or missing text shows no hint. */
  text?: string | undefined;
  /** A single element; it is described by the hint for assistive technology. */
  children: ReactElement<{ "aria-describedby"?: string }>;
  /** For the wrapper that measures the control, e.g. `flex` to stretch it. */
  className?: string;
}

/**
 * The app's tooltip: a small styled hint just under a control while it is hovered, centered on it, kept inside the
 * window, and above the control when there is no room below. It hides as soon as the control moves (a scroll).
 */
export function HoverHint({ text, children, className }: HoverHintProps) {
  const id = useId();
  const anchor = useRef<HTMLSpanElement>(null);
  const timer = useRef<number | undefined>(undefined);
  /** The control the hint is shown under; `null` while hidden. */
  const [shownOn, setShownOn] = useState<HTMLElement | null>(null);
  const isShown = shownOn !== null;
  const hasText = text !== undefined && text !== "";

  const hide = useCallback(() => {
    window.clearTimeout(timer.current);
    setShownOn(null);
  }, []);

  const show = useCallback(() => {
    if (!hasText) return;
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      hideShownHint?.();
      hideShownHint = hide;
      setShownOn(anchor.current);
    }, hintDelayMs);
  }, [hasText, hide]);

  useEffect(() => {
    if (!isShown) return;
    window.addEventListener("scroll", hide, true);
    window.addEventListener("resize", hide);
    return () => {
      window.removeEventListener("scroll", hide, true);
      window.removeEventListener("resize", hide);
      if (hideShownHint === hide) hideShownHint = undefined;
    };
  }, [isShown, hide]);

  useEffect(
    () => () => {
      window.clearTimeout(timer.current);
    },
    [],
  );

  return (
    <span
      ref={anchor}
      className={cx("inline-flex", className)}
      onPointerEnter={show}
      onPointerLeave={hide}
      onPointerDown={hide}
    >
      {hasText ? cloneElement(children, { "aria-describedby": `${id}-hint` }) : children}
      {hasText && (
        <span id={`${id}-hint`} hidden>
          {text}
        </span>
      )}
      {shownOn && hasText && createPortal(<HintBubble text={text} anchor={shownOn} />, document.body)}
    </span>
  );
}

function HintBubble({ text, anchor }: { text: string; anchor: HTMLElement }) {
  const bubble = useRef<HTMLDivElement>(null);
  const [origin, setOrigin] = useState<{ left: number; top: number }>();

  useLayoutEffect(() => {
    const size = bubble.current?.getBoundingClientRect();
    if (!size) return;
    const control = anchor.getBoundingClientRect();
    const gap = cssLength("--hint-gap");
    const margin = cssLength("--hint-margin");
    const centered = control.left + control.width / 2 - size.width / 2;
    const left = Math.min(Math.max(centered, margin), window.innerWidth - margin - size.width);
    let top = control.bottom + gap;
    if (top + size.height > window.innerHeight - margin) top = control.top - gap - size.height;
    setOrigin({ left, top });
  }, [anchor, text]);

  return (
    <div
      ref={bubble}
      role="tooltip"
      aria-hidden
      style={origin ?? { left: 0, top: 0, visibility: "hidden" }}
      className="pointer-events-none fixed z-50 line-clamp-5 w-max max-w-(--hint-max-width) rounded-hint border border-glass-stroke bg-background px-8 py-4 text-caption font-medium text-text"
    >
      {text}
    </div>
  );
}
