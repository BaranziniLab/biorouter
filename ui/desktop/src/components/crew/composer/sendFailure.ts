import { CREW_NOT_SENT, isTransportFailure, isTransportText } from '../api/errors';
import { CrewHttpError } from '../crewApi';
import { refusalCopy } from '../dialogs/copy';
import { parseRefusal, refusalText } from '../dialogs/refusals';
import { crewActionCopy } from '../state/copy';
import { composerCopy } from './copy';

/**
 * A refused post in words for a person (QA M1, M5, R-2, FILES-F9).
 *
 * The composer printed the daemon's text after "Couldn't send.", and for a refusal the broker
 * passed on that text is the broker's own `code: text`: `invalid_params: message too long`,
 * `quota_exceeded: workspace logical state exceeds 16 MiB; …`, `storage_failed: restart and
 * recover before further mutations` (addressed to the one person who can restart the broker), or
 * `forbidden: attachment provenance cannot be dropped`. Each now has one sentence that says what
 * to do. The refusal is read the way every dialog reads one (`dialogs/refusals.ts`), so a
 * sentence the broker already wrote for a person is shown as it is, and a code this renderer does
 * not know is still shown as sent, never guessed at.
 */

/** What the words may name, from the view the post was sent from. */
export interface SendFailureContext {
  /** The workspace host in the authority form, or null when the viewer cannot see them. */
  host: string | null;
  /** The viewer hosts the workspace: they, not someone else, can restart it. */
  isHost: boolean;
  /** The one file the post carried, by the name the draft gave it; null for none or several. */
  fileName: string | null;
  /** The channel that file was shared in (`#name`), when this computer knows it. */
  fileChannel: string | null;
}

/** What a failure records: its words, and whether it was the link's failure. */
export interface SendFailure {
  text: string;
  code?: string;
  transport: boolean;
}

/** The broker took the host's disk full as a post was written (`request_denied`, os error 28). */
const DISK_FULL = /No space left on device/i;

function refusalWords(failure: Error, context: SendFailureContext): string {
  const message = failure.message;
  const refusal = parseRefusal(message);
  const brokerCode = failure instanceof CrewHttpError ? failure.brokerCode : undefined;
  const code = refusal.code ?? brokerCode ?? null;
  const sentence = refusal.sentence;
  if (code === 'invalid_params' && /^message too long\b/i.test(sentence))
    return composerCopy.tooLong;
  // For the host, a full disk and a failed write are two things to do: space freed first, or a
  // restart (MSG2-N6). A member asks the host either way.
  const diskFull = code === 'storage_full' || DISK_FULL.test(message);
  if (diskFull || code === 'storage_failed')
    return context.isHost
      ? diskFull
        ? composerCopy.diskFullHost
        : composerCopy.storageFailedHost
      : composerCopy.storageFailed(context.host);
  if (code === 'forbidden' && /^attachment provenance\b/i.test(sentence))
    return composerCopy.fileElsewhere(context.fileName, context.fileChannel);
  if (code === 'forbidden' && /^reference provenance\b/i.test(sentence))
    return composerCopy.pathElsewhere;
  if (code === 'channel_archived') return composerCopy.archivedRefusal;
  if (code === 'quota_exceeded') {
    // A quota the broker words for a person ("You have used your share…") stays as written, and a
    // full workspace reads the dialogs' sentence, the host's own when the host sent it (MSG2-N6).
    // Any other quota a post can meet (the message limit, a limit newer than this renderer) means
    // the same to the person: it is full.
    const words = refusalText(message, { isHost: context.isHost });
    if (words !== refusal.text) return words;
    return context.isHost ? refusalCopy.storageFullHost : refusalCopy.storageFull;
  }
  if (failure instanceof CrewHttpError && failure.code === CREW_NOT_SENT)
    return composerCopy.notSent;
  if (!brokerCode && isTransportText(message)) return composerCopy.notReached;
  if (code && refusal.text !== sentence) return refusalText(message, { isHost: context.isHost });
  return message || crewActionCopy.actionFallback;
}

/** A refused post's words and code, and whether the connection verifying again makes it stale. */
export function sendFailure(failure: unknown, context: SendFailureContext): SendFailure {
  if (!(failure instanceof Error)) return { text: crewActionCopy.actionFallback, transport: false };
  const code = failure instanceof CrewHttpError ? failure.code : undefined;
  return {
    text: refusalWords(failure, context),
    ...(code !== undefined ? { code } : {}),
    transport: isTransportFailure(failure),
  };
}

/**
 * Whether `body` is over the broker's limit for one message (`mutate_message_post`): 65,536 UTF-8
 * bytes, and twice that once JSON-escaped (a body of control characters grows sixfold). Checked
 * before a post is sent, so an over-long message never leaves the composer (QA M5).
 */
export const MESSAGE_MAX_BYTES = 65_536;
export function messageTooLong(body: string): boolean {
  const encoder = new TextEncoder();
  if (encoder.encode(body).length > MESSAGE_MAX_BYTES) return true;
  return encoder.encode(JSON.stringify(body)).length - 2 > 2 * MESSAGE_MAX_BYTES;
}
