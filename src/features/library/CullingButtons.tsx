import { CircleCheck, CircleDashed, CircleX } from "lucide-react";

import type { Culling, CullingAction } from "../../bindings";
import { cx } from "../../design/cx";
import { Button } from "../../design/Styles";
import { cullingHint } from "./selection";

/**
 * Keep, Reject and Clear as icon buttons (`CullingButtons`): on a selection bar for `count` results, or on a result's
 * page, where the verdict it has (`current`) is lit.
 */
export function CullingButtons({
  count,
  current = null,
  onCull,
}: {
  count: number;
  current?: Culling | null;
  onCull: (action: CullingAction) => void;
}) {
  return (["keep", "reject", "clear"] as const).map((action) => {
    const isCurrent =
      (action === "keep" && current === "kept") || (action === "reject" && current === "rejected");
    return (
      <Button
        key={action}
        variant="icon"
        icon={{ keep: CircleCheck, reject: CircleX, clear: CircleDashed }[action]}
        title={cullingHint(action, count)}
        hint={cullingHint(action, count)}
        aria-pressed={current === null ? undefined : isCurrent}
        className={cx(isCurrent && "text-accent")}
        onClick={() => {
          onCull(action);
        }}
      />
    );
  });
}
