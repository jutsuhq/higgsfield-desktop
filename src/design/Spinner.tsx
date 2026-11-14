import { LoaderCircle } from "lucide-react";

/** `ProgressView().controlSize(.small)`: keeps turning with Reduce Motion, as the system's does. */
export function Spinner({ label }: { label?: string }) {
  return (
    <span role="progressbar" aria-label={label} className="inline-flex text-muted">
      <LoaderCircle aria-hidden data-essential-motion size={16} className="animate-spin" />
    </span>
  );
}
