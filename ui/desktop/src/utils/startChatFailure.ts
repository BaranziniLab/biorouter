import { errorMessage, isConnectionError } from './conversionUtils';
import { USER_ACTION_REFUSAL_MARKER } from './userAction';

/**
 * What a person is told when `POST /agent/start` fails — on every surface that
 * starts a chat: the Home composer, a fresh tab's composer, a window opened for
 * a workflow, a launcher message, the "Ask Biorouter" buttons.
 *
 * The 2026-09-10 QA run (finding F1) found the Home composer swallowing this
 * failure whole: the typed text vanished, nothing appeared, and the only trace
 * was a console line. The daemon's refusal had been correct, and was written
 * for a model — "Do not retry", "ask the user to switch this chat" — so showing
 * it verbatim would have been the second half of the same bug. Hence one pure
 * mapping, shared by every caller, that picks words for a person and keeps the
 * daemon's own text in the toast's copyable details.
 *
 * Pure (no toast, no DOM) so the words are tested without rendering anything,
 * and so `toasts.tsx`, which itself starts chats, can use it without an import
 * cycle. Each caller hands the result to `toastError`.
 */
export type StartChatFailureNotice = {
  title: string;
  msg: string;
  /** The daemon's own words, behind the toast's "Copy error". */
  traceback: string;
};

export const START_CHAT_FAILED_TITLE = 'Failed to start chat';
export const BACKEND_DISCONNECTED_TITLE = 'Backend disconnected';

/**
 * The daemon refused to bind its private default because the request carried
 * no proof it came from a person, on a backend that holds a user-action key
 * (SD-12). The `serve` daemon holds none and binds its default, so this reaches
 * a person only on a desktop app pointed at a backend started elsewhere — the
 * case `NO_USER_PROOF_TOAST_MSG` in `ModelAndProviderContext` words the same way.
 *
 * ⚠ The route answers with an `ErrorResponse`, so under `throwOnError` the
 * thrown value is the parsed `{ message }` object — not the plain string
 * `isUserActionRefusal` tests for, which is `/agent/update_provider`'s shape.
 * Both are accepted, keyed on the marker. A real `Error` carrying the same words
 * is not a policy refusal, whatever it reads.
 */
export const isStartRefusedForWantOfProof = (error: unknown): boolean => {
  if (error instanceof Error) return false;
  const text =
    typeof error === 'string'
      ? error
      : typeof error === 'object' &&
          error !== null &&
          'message' in error &&
          typeof error.message === 'string'
        ? error.message
        : null;
  return text !== null && text.includes(USER_ACTION_REFUSAL_MARKER);
};

/** The sentence a failure notice ends with when the caller kept the message. */
export const MESSAGE_KEPT_SENTENCE = 'Your message was kept.';

// A sentence is over when it ends in one of these...
const SENTENCE_END = /[.!?\u2026]$/;
// ...possibly inside a closing quote or bracket: `said "stop."`, `(see the log.)`.
const CLOSERS = /["'\u201d\u2019)\]\u00bb]+$/;
// A trailing separator with nothing after it: `…for the new chat:`.
const DANGLING_SEPARATOR = /[:;,]$/;

/**
 * Put a sentence of ours after a daemon's message, punctuated as one paragraph.
 *
 * Daemon refusals carry no terminal period as often as they carry one — the
 * 1.90.4 toast read "…must be an HTTPS URL with a host Your message was kept."
 * — so a bare space is not enough, and neither is an unconditional period:
 *
 *   • already a finished sentence, including one closed inside a quote or a
 *     bracket, gets a space and nothing else;
 *   • a message ending in a code span, a quote around a value, a bracket or a
 *     word gets a period AFTER it — never inside the span or the quote, whose
 *     contents are the daemon's, not prose;
 *   • a dangling `:` `;` `,` is the end of a template with nothing substituted,
 *     so it becomes the period rather than sitting before one.
 *
 * Only the TOAST's words go through this. The daemon's text itself stays
 * untouched in the notice's `traceback`, behind "Copy error".
 */
export function appendSentence(message: string, sentence: string): string {
  const text = message.trimEnd();
  if (!text) return sentence;
  if (SENTENCE_END.test(text.replace(CLOSERS, ''))) return `${text} ${sentence}`;
  if (DANGLING_SEPARATOR.test(text)) return `${text.slice(0, -1)}. ${sentence}`;
  return `${text}. ${sentence}`;
}

/**
 * @param kept whether the caller has put the message back where the person can
 *   see it (the composer). Say so only when it is true.
 */
export function startChatFailureNotice(
  error: unknown,
  { kept }: { kept: boolean }
): StartChatFailureNotice {
  // `errorMessage` answers its default, not the text, for a bare string.
  const daemonText = typeof error === 'string' ? error : errorMessage(error);
  if (isConnectionError(error)) {
    return {
      title: BACKEND_DISCONNECTED_TITLE,
      msg: kept
        ? 'Biorouter could not reach its backend. Your message was kept - try again in a moment.'
        : 'Biorouter could not reach its backend. Try again in a moment.',
      traceback: daemonText,
    };
  }
  const keptSentence = kept ? ` ${MESSAGE_KEPT_SENTENCE}` : '';
  if (isStartRefusedForWantOfProof(error)) {
    return {
      title: START_CHAT_FAILED_TITLE,
      msg:
        'Biorouter is connected to a backend started outside the app, which could not confirm ' +
        'the request came from you, so it did not start a chat on its private model. To use a ' +
        `private model, start the chat in the Biorouter app.${keptSentence}`,
      traceback: daemonText,
    };
  }
  // Every other refusal on this route is already written for a person —
  // "Failed to configure the selected provider for the new chat: …" — so it is
  // shown as it came, rather than replaced by something vaguer. It is still the
  // traceback too, which is what puts "Copy error" on the toast.
  return {
    title: START_CHAT_FAILED_TITLE,
    msg: kept ? appendSentence(daemonText, MESSAGE_KEPT_SENTENCE) : daemonText,
    traceback: daemonText,
  };
}
