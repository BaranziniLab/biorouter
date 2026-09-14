import type { Message } from '../../api';

/**
 * Item 7 — the notice a stopped turn ends on, as the daemon stores it.
 *
 * ⚠ Mirrors `TURN_STOPPED_NOTICE` in `crates/biorouter/src/agents/stopped_turn.rs`,
 * and a Rust test reads this exact declaration. Change both or neither.
 *
 * The daemon writes it as an ordinary inline system notification (user-visible,
 * model-hidden) — the shape the planning gate's durable verdicts already use —
 * so the CLI's export and any reader that does not know this constant still show
 * it. The desktop recognises it only to draw it as the same quiet "Stopped."
 * line a confirmed Stop shows live (`ChatTurnStopped`).
 */
export const TURN_STOPPED_NOTICE = 'Stopped.';

/** Is this row the stored "this reply was stopped" notice? Exact text, nothing looser. */
export function isTurnStoppedNotice(message: Message | undefined): boolean {
  if (!message || message.role !== 'assistant' || message.content.length !== 1) return false;
  const [content] = message.content;
  return (
    content.type === 'systemNotification' &&
    content.notificationType === 'inlineMessage' &&
    content.msg === TURN_STOPPED_NOTICE
  );
}

/** Does the transcript already end on a stored stop notice? */
export function transcriptEndsStopped(messages: readonly Message[]): boolean {
  return isTurnStoppedNotice(messages[messages.length - 1]);
}

/**
 * Make a live transcript agree with what a Stop wrote to the store.
 *
 * `record` is `/agent/cancel`'s `stop_messages`: the rows the daemon stored for
 * the stopped turn, in order. A row whose id the view already holds replaces it
 * — the streamed reply is shown as it was stored, prose only, exactly as a
 * reload will show it — and a row the view lacks is appended. Appended rather
 * than inserted: every row in a stop record was written after anything this view
 * could already hold from that turn.
 *
 * Returns `messages` itself when nothing changes, so a caller can skip a render.
 */
export function mergeStopRecord(messages: Message[], record: readonly Message[]): Message[] {
  if (record.length === 0) return messages;
  const next = [...messages];
  let changed = false;
  for (const row of record) {
    const at = row.id ? next.findIndex((message) => message.id === row.id) : -1;
    if (at >= 0) {
      if (JSON.stringify(next[at]) !== JSON.stringify(row)) {
        next[at] = row;
        changed = true;
      }
    } else {
      next.push(row);
      changed = true;
    }
  }
  return changed ? next : messages;
}
