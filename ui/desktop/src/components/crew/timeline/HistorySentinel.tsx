import { useEffect, useRef } from 'react';
import { Button } from '../../ui/button';
import { timelineCopy } from './copy';

/**
 * The top of a full page of history. Older messages load by themselves when
 * this row scrolls into view, through an `IntersectionObserver` rooted on the
 * log's scroller, and the "Older messages" button (pinned) is the keyboard path
 * and the path wherever there is no observer (jsdom has none, so tests click).
 *
 * The observer only REPORTS; `onReached` decides. The timeline arms it after
 * the reader scrolls up, so a page that lands with this row in view does not
 * load the next one by itself.
 */
export function HistorySentinel({
  loading,
  disabled,
  onLoad,
  onReached,
  root,
}: {
  loading: boolean;
  disabled: boolean;
  onLoad(): void;
  onReached(): void;
  /** The scroller the row is observed in; the viewport when absent. */
  root: () => Element | null;
}) {
  const row = useRef<HTMLDivElement>(null);
  const reached = useRef(onReached);
  useEffect(() => {
    reached.current = onReached;
  }, [onReached]);

  useEffect(() => {
    const element = row.current;
    if (!element || typeof IntersectionObserver === 'undefined') return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) reached.current();
      },
      { root: root(), rootMargin: '160px 0px 0px 0px' }
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [root]);

  return (
    <div ref={row} className="crew-history-sentinel">
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="text-text-muted"
        disabled={disabled || loading}
        onClick={onLoad}
      >
        {loading ? timelineCopy.loadingOlder : timelineCopy.older}
      </Button>
    </div>
  );
}
