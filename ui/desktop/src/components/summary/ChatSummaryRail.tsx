import { useEffect, useRef, useState } from 'react';
import { ChatSummary, type ChatSummaryProps } from '../ChatSummary';
import { summaryCopy } from './copy';
// Imported statically by BaseChat (through this module), so the rail's grid
// rules are in the document before the first chat renders: a lazily loaded
// stylesheet would leave the split box unstyled until the module arrived.
import './summaryRail.css';

export interface ChatSummaryRailProps extends ChatSummaryProps {
  /** The id the header button's `aria-controls` names. */
  id: string;
  /** The grid cell this rail occupies; BaseChat passes `"rail"`. */
  'data-preview-area': 'rail';
  /**
   * The rail appeared because of geometry (a resize, a preview, a tab switch):
   * it mounts at rest. Read once, at mount; a later change never replays it.
   */
  still: boolean;
  /** `closed` while the card plays its exit after the person closed it. */
  state: 'open' | 'closed';
  /** The exit has finished: the grid may drop the column. */
  onExited: () => void;
}

/**
 * The docked Chat summary: a Codex-style card in the grid's `rail` column, to
 * the right of the conversation and under the chat header (rung 0 of the yield
 * ladder decides whether it fits; `hooks/useSummaryRail.ts` holds its state).
 *
 * Motion is the card's own: it slides 12px and fades in after a 60ms delay when
 * the person opens it or the first turn starts, and fades out sliding 8px when
 * they close it. Everything geometric (a resize, the sidebar, a preview, a tab
 * switch) snaps.
 */
export function ChatSummaryRail({
  id,
  'data-preview-area': area,
  still,
  state,
  onExited,
  ...summary
}: ChatSummaryRailProps) {
  const [mountedStill] = useState(still);
  // Once the entrance has played (or was skipped), it must never replay: a
  // resize zeroes animations while it lasts (`[data-motion-layout]`), and an
  // animation that comes back afterwards would start again from the top.
  const [entered, setEntered] = useState(still);
  const [previousState, setPreviousState] = useState(state);
  if (previousState !== state) {
    setPreviousState(state);
    // Closing re-arms the entrance, so a reopen during the exit slides back in.
    if (state === 'closed') setEntered(false);
  }

  // `animationend`, or `animationcancel` when a resize removed the animation
  // (React has no handler for the latter, so both are listened to natively).
  const cardRef = useRef<HTMLDivElement>(null);
  const doneRef = useRef<() => void>(() => {});
  doneRef.current = () => {
    if (state === 'closed') onExited();
    else setEntered(true);
  };
  useEffect(() => {
    const card = cardRef.current;
    if (!card) return;
    const handle = (event: Event) => {
      if (event.target === card) doneRef.current();
    };
    card.addEventListener('animationend', handle);
    card.addEventListener('animationcancel', handle);
    return () => {
      card.removeEventListener('animationend', handle);
      card.removeEventListener('animationcancel', handle);
    };
  }, []);

  return (
    <aside
      id={id}
      data-preview-area={area}
      data-rail-still={mountedStill ? '' : undefined}
      aria-label={summaryCopy.title}
      className="br-summary-rail"
    >
      <div
        className="br-summary-card"
        data-state={state}
        data-entered={entered ? '' : undefined}
        data-motion-layout=""
        ref={cardRef}
      >
        <ChatSummary {...summary} />
      </div>
    </aside>
  );
}
