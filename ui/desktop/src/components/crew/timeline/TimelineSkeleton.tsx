import { useEffect, useState } from 'react';
import { Skeleton } from '../../ui/skeleton';

/** The skeleton waits this long, so a fast load never flashes it. */
export const SKELETON_DELAY_MS = 150;

const ROWS = [
  {
    name: 'crew-skeleton-name-short',
    lines: ['crew-skeleton-line-long', 'crew-skeleton-line-mid'],
  },
  { name: 'crew-skeleton-name-long', lines: ['crew-skeleton-line-mid'] },
  {
    name: 'crew-skeleton-name-short',
    lines: ['crew-skeleton-line-long', 'crew-skeleton-line-short'],
  },
];

/**
 * What a channel shows while its messages load (baseline critique: the empty
 * "Welcome to #channel" state used to stand in for two or three seconds, which
 * read as an empty channel). Nothing for the first 150ms, then message-shaped
 * placeholders; the log carries `aria-busy` meanwhile and this names the wait
 * for a screen reader.
 */
export function TimelineSkeleton({ label }: { label: string }) {
  const [shown, setShown] = useState(false);
  useEffect(() => {
    const timer = window.setTimeout(() => setShown(true), SKELETON_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, []);
  return (
    <div className="crew-timeline-skeleton" data-shown={shown ? 'true' : undefined}>
      <span className="sr-only">{label}</span>
      {shown &&
        ROWS.map((row, index) => (
          <div key={index} className="crew-skeleton-row" aria-hidden="true">
            <Skeleton className="crew-skeleton-avatar" />
            <div className="crew-skeleton-text">
              <Skeleton className={row.name} />
              {row.lines.map((line, lineIndex) => (
                <Skeleton key={lineIndex} className={line} />
              ))}
            </div>
          </div>
        ))}
    </div>
  );
}
