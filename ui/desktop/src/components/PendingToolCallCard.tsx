import './tool-call.css';
import React from 'react';
import { ToolIconWithStatus } from './ToolCallStatusIndicator';
import { getToolCallIcon } from '../utils/toolIconMapping';
import { toolIdentifierToTitleCase } from '../utils';
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
  return (
    <div className="br-tool-pending mt-3 text-text-muted" data-testid="pending-tool-call" data-tool-id={pending.id}>
      <div className="flex h-6 items-center">
        <span className="flex min-w-0 max-w-full items-center gap-2 overflow-hidden font-sans text-sm leading-6">
          <ToolIconWithStatus
            ToolIcon={getToolCallIcon(pending.name)}
            status="loading"
            className="mt-px"
          />
          <span className="br-tool-running min-w-0 flex-1 truncate">
            <span>Preparing</span> <span>{toolSummary}</span>
          </span>
        </span>
      </div>
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
