/**
 * A muted video that loops from the start while shown, filling its frame: hover previews of styles and clips.
 * It never takes the mouse, so the tile under it keeps its clicks and hover. A new `src` starts from the beginning.
 */
export function LoopingVideo({ src }: { src: string }) {
  return (
    <video
      key={src}
      src={src}
      muted
      loop
      autoPlay
      playsInline
      aria-hidden
      className="pointer-events-none size-full object-cover"
    />
  );
}
