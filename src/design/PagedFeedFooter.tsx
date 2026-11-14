import { useEffect, useRef } from "react";

import { cx } from "./cx";
import { Spinner } from "./Spinner";
import { Button } from "./Styles";

interface PagedFeedFooterProps {
  /** A failed page's title, with Try Again. */
  failure?: { title: string } | undefined;
  hasMore: boolean;
  /** How many items are shown; each new count lets the spinner ask for the next page again. */
  count: number;
  /** The spinner stands for a first page that something else already shows, such as skeleton tiles. */
  hidesSpinner?: boolean;
  loadMore: () => void | Promise<void>;
}

/**
 * The end of a list read page by page: a failed page's title with Try Again, else a spinner that loads the next
 * page once it is scrolled into view, else nothing. Place it after the items.
 */
export function PagedFeedFooter({
  failure,
  hasMore,
  count,
  hidesSpinner = false,
  loadMore,
}: PagedFeedFooterProps) {
  if (failure) {
    return (
      <div className="flex items-center gap-small pt-large">
        <span className="text-muted">{failure.title}</span>
        <Button variant="secondary" title="Try Again" onClick={() => void loadMore()} />
      </div>
    );
  }
  if (!hasMore) return null;
  return <LoadMoreSpinner count={count} isHidden={hidesSpinner} loadMore={loadMore} />;
}

function LoadMoreSpinner({
  count,
  isHidden,
  loadMore,
}: {
  count: number;
  isHidden: boolean;
  loadMore: () => void | Promise<void>;
}) {
  const footer = useRef<HTMLDivElement>(null);
  const load = useRef(loadMore);
  useEffect(() => {
    load.current = loadMore;
  }, [loadMore]);

  // `.task(id: count)`: once on screen, and again for every new count while it stays there.
  useEffect(() => {
    const element = footer.current;
    if (!element) return;
    if (typeof IntersectionObserver === "undefined") {
      void load.current();
      return;
    }
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) {
        observer.disconnect();
        void load.current();
      }
    });
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, [count]);

  return (
    <div ref={footer} className={cx("flex justify-center pt-large", isHidden && "opacity-0")}>
      <Spinner />
    </div>
  );
}
