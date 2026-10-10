import React from 'react';
import { toolGlyphFor } from '../utils/toolGlyph';
import { toolIdentifierToTitleCase } from '../utils';
import { TranscriptRow } from './TranscriptRow';
import { TOOL_ROW_COPY } from './toolCallCopy';
import type { PendingToolCallView } from '../hooks/chatStreamStore';

/**
 * §6.1b — a skeleton card for a tool call the model has begun emitting whose
 * arguments have not finished streaming. Rendered from
 * `snapshot.pendingToolCalls` (never from `messages`), so it can never carry a
 * dispatchable request. It shows the tool NAME the instant it is known — seconds
 * before the arguments finish — and is replaced by the real
 * `ToolCallWithResponse` card the moment the authoritative request lands (the
 * store removes the pending entry by id).
 *
 * Deliberately does NOT render partial arguments: a truncated JSON fragment is
 * meaningless to a human and risks implying the call is more complete than it
 * is. The card is a placeholder, not a preview of unparsed bytes.
 */
export const PendingToolCallCard: React.FC<{ pending: PendingToolCallView }> = ({ pending }) => {
  const toolSummary = toolIdentifierToTitleCase(pending.name.split('__').pop() ?? pending.name);
  // No arguments yet, so the ladder answers from the name alone.
  const glyph = toolGlyphFor(pending.name);
  // The same 28px line the real call will draw, static (nothing to open yet)
  // and breathing like any running row, so the swap to the real card is a
  // change of words, not of shape.
  return (
    <div className="mt-1" data-testid="pending-tool-call" data-tool-id={pending.id}>
      <TranscriptRow
        icon={glyph.Icon}
        glyph={glyph.kind}
        label={
          <>
            <span>{TOOL_ROW_COPY.preparing}</span> <span>{toolSummary}</span>
          </>
        }
        running
        statusLabel={TOOL_ROW_COPY.statusLabel('loading')}
      />
    </div>
  );
};

/** Renders the current set of pending tool-call skeletons, in arrival order. */
export const PendingToolCallList: React.FC<{ pending: PendingToolCallView[] }> = ({ pending }) => {
  if (pending.length === 0) return null;
  return (
    <>
      {pending.map((p) => (
        <PendingToolCallCard key={p.id} pending={p} />
      ))}
    </>
  );
};
