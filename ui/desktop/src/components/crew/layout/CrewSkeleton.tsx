import { useEffect, useState } from 'react';
import { Skeleton } from '../../ui/skeleton';

/** The skeleton waits this long, so a fast load never flashes it (ui-redesign-spec, `loading`). */
export const CREW_SKELETON_DELAY_MS = 150;

/** True once `CREW_SKELETON_DELAY_MS` has passed since mount. */
function useDelayed(): boolean {
  const [shown, setShown] = useState(false);
  useEffect(() => {
    const timer = window.setTimeout(() => setShown(true), CREW_SKELETON_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, []);
  return shown;
}

const SIDEBAR_SECTIONS = [3, 3] as const;
const MESSAGE_BLOCKS = [
  ['crew-frame-bone-name', 'crew-frame-bone-long', 'crew-frame-bone-mid'],
  ['crew-frame-bone-name-wide', 'crew-frame-bone-mid'],
  ['crew-frame-bone-name', 'crew-frame-bone-long', 'crew-frame-bone-short'],
  ['crew-frame-bone-name-wide', 'crew-frame-bone-long'],
] as const;

/**
 * The Crew sidebar's shape while the saved workspaces load: two section headers and six rows, on
 * the sidebar's own ground. Decoration only; the main area names the wait.
 */
export function SidebarSkeleton() {
  const shown = useDelayed();
  return (
    <div className="crew-frame-sidebar-skeleton" aria-hidden="true" data-shown={shown || undefined}>
      <div className="crew-frame-band" />
      {shown &&
        SIDEBAR_SECTIONS.map((rows, section) => (
          <div key={section} className="crew-frame-bone-section">
            <Skeleton className="crew-frame-bone-heading" />
            {Array.from({ length: rows }, (_, row) => (
              <Skeleton key={row} className="crew-frame-bone-row" />
            ))}
          </div>
        ))}
    </div>
  );
}

/**
 * Four message-shaped blocks in the chat column, after the same delay. `label` is read by a
 * screen reader at once; the blocks are decoration.
 */
export function MessagesSkeleton({ label }: { label: string }) {
  const shown = useDelayed();
  return (
    <div className="crew-frame-messages-skeleton" role="status">
      <span className="sr-only">{label}</span>
      {shown &&
        MESSAGE_BLOCKS.map((lines, block) => (
          <div key={block} className="crew-frame-bone-message" aria-hidden="true">
            <Skeleton className="crew-frame-bone-avatar" />
            <div className="crew-frame-bone-lines">
              {lines.map((line, index) => (
                <Skeleton key={index} className={line} />
              ))}
            </div>
          </div>
        ))}
    </div>
  );
}
