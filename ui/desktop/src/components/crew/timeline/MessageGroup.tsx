import { useId } from 'react';
import type { TimelineGroup } from './groupMessages';
import { MessageRow, TraceRow } from './MessageRow';

/**
 * One author's run of messages: an `<article>` named by its author and time.
 * The first row carries the head; the rest are continuations. An agent's group
 * reads as the agent — a square Bot tile, "Alice Chen's agent" or "Your agent",
 * `@alice` and an "Agent" badge — never as the person who owns it.
 */
export function MessageGroup({ group }: { group: TimelineGroup }) {
  const author = useId();
  const time = useId();
  const ids = { author, time };
  return (
    <article
      className="crew-message-group"
      data-agent={group.agent ? 'true' : undefined}
      aria-labelledby={`${author} ${time}`}
    >
      {group.entries.map((entry) =>
        entry.kind === 'trace' ? (
          <TraceRow key={entry.key} group={group} entry={entry} ids={ids} />
        ) : (
          <MessageRow key={entry.key} group={group} entry={entry} ids={ids} />
        )
      )}
    </article>
  );
}
