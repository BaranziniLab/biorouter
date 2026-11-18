import type { Message } from '../../api';

/**
 * The note a turn the person cancelled ends on, as the daemon stores it (owner
 * ruling 2026-11-17: a record, not a guard). The daemon stores it as the agent's
 * own text, read by the person AND by the model on its next turn, because every
 * wire format drops a system notification.
 *
 * ⚠ Mirrors `TURN_STOPPED_NOTICE` in `crates/biorouter/src/agents/stopped_turn.rs`,
 * and a Rust test reads this exact declaration. Change both or neither.
 *
 * A reader that does not know this constant (the CLI's export, an older desktop)
 * shows it as the agent's line it is. The desktop recognises it to draw it as
 * the same quiet line a confirmed Stop shows live (`ChatTurnStopped`), in its
 * own words.
 */
export const TURN_STOPPED_NOTICE = 'The person canceled this turn before it finished.';

/**
 * The short line a turn ends on when something other than the person cancelled
 * it (another agent, a monitor, the orphan reaper), and the one every stopped
 * turn ended on before the cancel note: an inline system notification the model
 * never reads. Chats stored before the note still end on it.
 *
 * ⚠ Mirrors `TURN_ENDED_NOTICE` in the same Rust file.
 */
export const TURN_ENDED_NOTICE = 'Stopped.';

/**
 * The words a stored stop row is drawn with, or `undefined` when the row is not
 * one: the cancel note (the agent's text, exactly the note, nothing else) or the
 * short line (an inline notice, exactly `Stopped.`). Exact text, nothing looser.
 *
 * The note's words are compared trimmed: after the prose a stopped reply had
 * streamed, the daemon stores it as a new paragraph, because the model reads
 * the two rows as one reply.
 */
export function turnStoppedNoticeText(message: Message | undefined): string | undefined {
  if (!message || message.role !== 'assistant' || message.content.length !== 1) return undefined;
  const [content] = message.content;
  if (content.type === 'text' && content.text.trim() === TURN_STOPPED_NOTICE) {
    return TURN_STOPPED_NOTICE;
  }
  if (
    content.type === 'systemNotification' &&
    content.notificationType === 'inlineMessage' &&
    content.msg === TURN_ENDED_NOTICE
  ) {
    return content.msg;
  }
  return undefined;
}

/** Is this row a stored "this turn was stopped" row? */
export function isTurnStoppedNotice(message: Message | undefined): boolean {
  return turnStoppedNoticeText(message) !== undefined;
}

/**
 * The row as the transcript's rhythm should see it: a stop row is a notice line,
 * never an answer whose floating actions reach into the row above. The cancel
 * note is stored as text so the model reads it; for spacing it is the notice it
 * draws as.
 */
export function asTranscriptNotice(message: Message): Message {
  const text = turnStoppedNoticeText(message);
  if (text === undefined || message.content[0]?.type !== 'text') return message;
  return {
    ...message,
    content: [{ type: 'systemNotification', notificationType: 'inlineMessage', msg: text }],
  } as Message;
}

/** Does the transcript already end on a stored stop notice? */
export function transcriptEndsStopped(messages: readonly Message[]): boolean {
  return isTurnStoppedNotice(messages[messages.length - 1]);
}

/** A later turn's Stop must not change the earlier turn's approval cards. */
export function messageTurnWasStopped(messages: readonly Message[], index: number): boolean {
  if (index < 0) return false;
  for (let next = index + 1; next < messages.length; next++) {
    const message = messages[next];
    if (isTurnStoppedNotice(message)) return true;
    if (
      message.role === 'user' &&
      message.metadata.userVisible &&
      message.content.some((content) => content.type !== 'toolResponse')
    ) {
      return false;
    }
  }
  return false;
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
