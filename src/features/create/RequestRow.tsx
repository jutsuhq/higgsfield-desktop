import { CircleCheck, CircleX, Hand, TriangleAlert } from "lucide-react";

import type { ImageRequestRow_Serialize } from "../../bindings";
import { Spinner } from "../../design/Spinner";
import { Button, Well } from "../../design/Styles";

/**
 * One image request on a line (`ImageRunRow`, `RequestRow`): a spinner and Cancel while it runs, Review… while an
 * agent's request waits for the person, else how it ended. Its images show as placeholders in the strip, so the row
 * needs no progress bar.
 */
export function RequestRow({
  row,
  onCancel,
  onReview,
}: {
  row: ImageRequestRow_Serialize;
  onCancel: () => void;
  onReview?: () => void;
}) {
  return (
    <Well className="flex items-center gap-10 px-medium py-10">
      <StatusIcon status={row.status} />
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        <p className="m-0 truncate text-small text-text">{row.title}</p>
        {row.subtitle !== null && <p className="m-0 truncate text-caption text-faint">{row.subtitle}</p>}
        {row.detail !== null && (
          <p
            className={
              row.status === "failed" || row.status === "waiting"
                ? "m-0 text-caption text-warning"
                : row.status === "working"
                  ? "m-0 text-caption text-muted"
                  : "m-0 text-caption text-faint"
            }
          >
            {row.detail}
          </p>
        )}
      </div>
      {row.status === "waiting" && onReview !== undefined && (
        <Button
          variant="secondary"
          title="Review…"
          hint="Review the price and send it, or cancel it"
          onClick={onReview}
        />
      )}
      {row.canCancel && (
        <Button variant="secondary" title="Cancel" hint="Cancel request" onClick={onCancel} />
      )}
    </Well>
  );
}

function StatusIcon({ status }: { status: ImageRequestRow_Serialize["status"] }) {
  switch (status) {
    case "working":
      return <Spinner />;
    case "finished":
      return <CircleCheck aria-hidden size={16} className="text-accent" />;
    case "failed":
      return <TriangleAlert aria-hidden size={16} className="text-warning" />;
    case "canceled":
      return <CircleX aria-hidden size={16} className="text-faint" />;
    case "waiting":
      return <Hand aria-hidden size={16} className="text-warning" />;
  }
}
