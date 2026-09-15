import { Message } from '../api';
import { ChatState } from '../types/chatState';
import {
  getElicitationContent,
  getSecretRequestContent,
  getToolConfirmationContent,
  getToolRequests,
  getToolResponses,
} from '../types/message';

export type TrailingPhase = 'thinking' | 'running' | 'compacting' | 'steering' | 'steerQueued';

export interface TrailingActivity {
  phase: TrailingPhase;
  /** Label shown next to the pulse. Never contains the elapsed time. */
  label: string;
  /**
   * The user's own words, echoed back while a soft interrupt is in flight.
   * Only set for phases 'steering' and 'steerQueued' — every other phase
   * narrates the AGENT, and putting user text in them would misattribute it.
   */
  steerText?: string;
  /**
   * Client-clock ms the indicator counts from. `undefined` means "we have no
   * trustworthy origin" (a reload mid-turn) — render the pulse with NO timer
   * rather than counting from mount, which would be a fabricated number.
   */
  since?: number;
}

export interface DeriveTrailingActivityArgs {
  messages: Message[];
  /** chatState !== Idle. False on any historical/read-only render. */
  isTurnActive: boolean;
  chatState?: ChatState;
  /** Store-owned client timestamps. */
  turnStartedAt?: number;
  lastMessageAt?: number;
  /**
   * BR-61 — a soft interrupt the store has accepted but the agent has not yet
   * consumed. Set optimistically the instant the POST is issued and cleared
   * when the agent echoes the text back (or the steer is rejected), so the
   * window between the user's click and the agent's next output is never blank.
   */
  pendingSteer?: PendingSteer;
}

export interface PendingSteer {
  text: string;
  /** Client clock (ms) the steer was issued at — the indicator's timer origin. */
  since: number;
  /**
   * What the daemon said the accepted steer is waiting behind (its
   * `SteerWaiting` frame): a running tool, named when it could be, or a card.
   */
  waitingOn?: { reason: 'tool' | 'approval' | 'delegation'; toolName?: string };
}

const isToolResponseOnly = (m: Message) =>
  m.content.length > 0 && m.content.every((c) => c.type === 'toolResponse');

const hasVisibleText = (m: Message) =>
  m.content.some((c) => c.type === 'text' && c.text.trim().length > 0);

// ⚠ The `toolConfirmationRequest` arm this replaced was DEAD — nothing in
// `crates/` constructs that variant — so an approval card never satisfied this
// predicate, the guard below never fired, and a card-carrying message (no tool
// requests, no visible text) fell through to "Thinking". The result was a
// running clock under an approval card for the whole of its TTL.
//
// Issue #117: a credential card parks the turn exactly as an elicitation does.
// Omitting it would leave the composer reporting "still working" while the
// install sits waiting on a dialog.
const awaitsToolConfirmation = (m: Message) =>
  getToolConfirmationContent(m) !== undefined ||
  getElicitationContent(m) !== undefined ||
  getSecretRequestContent(m) !== undefined;

/** The name of a tool the last assistant message requested and nothing has
 * answered yet, when there is one. */
function outstandingToolName(last: Message | undefined): string | undefined {
  if (!last || last.role !== 'assistant') return undefined;
  const answered = new Set(getToolResponses(last).map((r) => r.id));
  const request = getToolRequests(last).find((r) => !answered.has(r.id));
  if (!request) return undefined;
  // The daemon sends either the wrapped `ToolResult` shape or the bare call.
  const call = request.toolCall as
    | { status?: string; value?: { name?: string }; name?: string }
    | undefined;
  return call?.status === 'success' ? call.value?.name : call?.name;
}

/**
 * Derives the trailing "still working" indicator from facts alone.
 *
 * This is a PURE function on purpose. `ProgressiveMessageList`'s render
 * callback has `messages` in its deps, whose identity changes on every stream
 * event, so nothing downstream can memoise across a turn — any component state
 * here (a mount-time timestamp especially) would be reset by the list's churn
 * and would lie after a reload. Every input is either message content or a
 * store-owned client timestamp.
 */
export function deriveTrailingActivity({
  messages,
  isTurnActive,
  chatState,
  turnStartedAt,
  lastMessageAt,
  pendingSteer,
}: DeriveTrailingActivityArgs): TrailingActivity | null {
  // (a) Historical / read-only / finished turn. The single most important gate:
  //     SessionHistoryView never passes isStreamingMessage, so this is false
  //     there and a replayed session can never show a live indicator.
  if (!isTurnActive) return null;

  const last = messages[messages.length - 1];

  // (a2) The turn is waiting on the PERSON — a card is on screen, or the
  //      conversation is still loading. This OUTRANKS a pending steer (D3):
  //      "Steering the current turn · 4m 1s" plus "Still working…" under an
  //      unanswered Run Shell? card told the user the agent was busy while it
  //      was waiting for them, and the same wait with no steer showed nothing
  //      at all. A steer never answers a card, so say honestly what it waits
  //      behind — with no clock and no nudge, because nothing is working.
  const waitsOnTheUser =
    chatState === ChatState.WaitingForUserInput ||
    (last !== undefined && awaitsToolConfirmation(last)) ||
    pendingSteer?.waitingOn?.reason === 'approval';
  if (chatState === ChatState.LoadingConversation) return null;
  if (waitsOnTheUser) {
    if (!pendingSteer) return null;
    return {
      phase: 'steerQueued',
      label: 'Your message will be added after you answer the card',
      steerText: pendingSteer.text,
      since: undefined,
    };
  }

  // (a3) BR-61 — a soft interrupt is in flight. This outranks every branch
  //      below, including the ones that deliberately return null (streaming
  //      prose): those all narrate what the AGENT is doing, and none of them
  //      would tell the user that the message that just left their composer
  //      was heard. Without this the press is followed by seconds of nothing,
  //      which reads as a dropped input.
  if (pendingSteer) {
    const runningTool =
      pendingSteer.waitingOn?.reason === 'tool'
        ? (pendingSteer.waitingOn.toolName ?? 'the running tool')
        : outstandingToolName(last);
    return {
      phase: 'steering',
      label: runningTool
        ? `Your message will be added when ${runningTool} finishes`
        : 'Steering the current turn',
      steerText: pendingSteer.text,
      since: pendingSteer.since,
    };
  }

  if (!last) return null;

  const since = lastMessageAt ?? turnStartedAt;

  if (chatState === ChatState.Compacting) {
    return { phase: 'compacting', label: 'Compacting the chat', since };
  }

  // (c) THE CASE THIS FEATURE EXISTS FOR: a tool just returned, the model is
  //     mid round-trip, and nothing at all is on screen. A tool-response
  //     message is role 'user' with only toolResponse content, which
  //     ProgressiveMessageList renders as an empty div.
  if (last.role === 'user' && isToolResponseOnly(last)) {
    return { phase: 'thinking', label: 'Working on the result', since };
  }

  // (d) The assistant message with tool calls is still last and some calls have
  //     no response yet -> tools are executing. The cards carry their own
  //     status, so we only label; `since: undefined` suppresses our timer to
  //     keep exactly one clock on screen at a time.
  if (last.role === 'assistant') {
    const requests = getToolRequests(last);
    if (requests.length > 0) {
      const answered = new Set(getToolResponses(last).map((r) => r.id));
      const outstanding = requests.filter((r) => !answered.has(r.id)).length;
      if (outstanding > 0) {
        return {
          phase: 'running',
          label: outstanding === 1 ? 'Running the tool' : `Running ${outstanding} tools`,
          since: undefined,
        };
      }
    }
    // (e) Assistant prose is streaming in. The visible text IS the feedback.
    if (hasVisibleText(last)) return null;
    return { phase: 'thinking', label: 'Thinking', since };
  }

  // (f) User just submitted; nothing back yet.
  return { phase: 'thinking', label: 'Thinking', since };
}
