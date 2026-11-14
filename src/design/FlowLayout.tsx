import type { HTMLAttributes } from "react";

import { cx } from "./cx";

/** The spacings the Swift views pass: 4, 6 (the default) and 8. */
const gapClass = { "x-small": "gap-x-small", "6": "gap-6", small: "gap-small" } as const;

/** Lays views out left to right and wraps onto a new line when the row is full, like words in a paragraph. */
export function FlowLayout({
  spacing = "6",
  className,
  ...rest
}: HTMLAttributes<HTMLDivElement> & { spacing?: keyof typeof gapClass }) {
  return <div className={cx("flex flex-wrap items-start", gapClass[spacing], className)} {...rest} />;
}
