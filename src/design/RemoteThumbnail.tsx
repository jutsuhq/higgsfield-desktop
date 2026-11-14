import { useState } from "react";

import { cx } from "./cx";

interface RemoteThumbnailProps {
  /** Only an https link loads; anything else shows nothing. */
  url?: string | null | undefined;
  /** `fill` crops to the frame, as tiles do; `fit` shows the whole picture. */
  contentMode?: "fill" | "fit";
  className?: string;
}

/** A remote picture filling its frame, faded in once it has loaded; a link that fails to load shows nothing. */
export function RemoteThumbnail({ url, contentMode = "fill", className }: RemoteThumbnailProps) {
  const [loaded, setLoaded] = useState<string>();
  const [failed, setFailed] = useState<string>();
  if (!url || !isHttps(url) || failed === url) return null;
  return (
    <img
      src={url}
      alt=""
      loading="lazy"
      decoding="async"
      onLoad={() => {
        setLoaded(url);
      }}
      onError={() => {
        setFailed(url);
      }}
      className={cx(
        "size-full",
        contentMode === "fill" ? "object-cover" : "object-contain",
        loaded === url ? "animate-fade-in" : "invisible",
        className,
      )}
    />
  );
}

function isHttps(url: string): boolean {
  try {
    return new URL(url).protocol === "https:";
  } catch {
    return false;
  }
}
