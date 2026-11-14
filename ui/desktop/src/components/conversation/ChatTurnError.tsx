import { createContext } from 'react';
import { AlertTriangle, Info } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Disclosure } from '../ui/disclosure';
import { TranscriptRow } from '../TranscriptRow';
import { InfoTip } from '../ui/info-tip';
import { transcriptCopy } from '../transcript/copy';
import type { Message } from '../../api';
import type { ChatTurnErrorData } from '../../types/turnError';
import { isConnectionError } from '../../utils/conversionUtils';
import { crewTurnRefusalOf } from '../crew/access/crewTurnRefusal';
import { cn } from '../../utils';
import { opensTurn } from '../../utils/failedTurn';
import {
  REPETITION_STOP_FALLBACK,
  REPETITION_STOP_TITLE,
  TOOL_LOOP_CODE,
  repetitionStopNotice,
} from '../../utils/repetitionStop';
import { foldForHelp } from '../../utils/proseFold';

/** The info icon's name beside a folded error message: "About this error". */
const ERROR_TIP_NAME = 'this error';

export interface ChatTurnErrorPresentation {
  title: string;
  message: string;
  details?: string;
}

const PROVIDER_TITLES: Record<string, string> = {
  auth: 'Model authentication failed',
  context_length: 'Model context limit exceeded',
  invalid_request: 'Model rejected the request',
  model_unavailable: 'Model unavailable',
  policy: 'Request blocked by provider policy',
  quota: 'Model quota exceeded',
  rate_limit: 'Model rate limit reached',
  server: 'Model provider unavailable',
};

function decodeProviderMessage(value: string): string {
  try {
    return JSON.parse(`"${value}"`) as string;
  } catch {
    return value.replace(/\\"/g, '"').replace(/\\n/g, '\n');
  }
}

function providerMessage(value: string): string | undefined {
  const encoded = value.match(/"message":\s*String\("((?:\\.|[^"\\])*)"\)/)?.[1];
  return encoded ? decodeProviderMessage(encoded) : undefined;
}

// Codes whose transport failure is a fetch to biorouterd itself failing (the
// app's own backend), NOT the model provider. These get "backend is
// restarting" copy so the user checks the right thing — retrying in a moment
// rather than hunting through provider settings for a key that is fine.
const BACKEND_CODES = new Set(['session_load_unreachable', 'agent_load_failed', 'submit_error']);

// A dropped/closed stream mid-response, as opposed to a connection that never
// opened. Distinct copy: the request DID start, so "retry to continue".
const MIDSTREAM_CODES = new Set(['stream_error', 'stream_interrupted']);

// M2 — the two endings the USER caused. Both arrive as `scope: 'internal'`,
// which is the scope of a turn that died on its own, so without their own
// titles they read as "Model turn ended unexpectedly" — the app blaming the
// model for a Stop the user pressed. The store is what distinguishes them (only
// it knows a Stop was outstanding); this map is what names them.
const STOP_TITLES: Record<string, string> = {
  turn_stopped_by_user: 'Turn stopped',
  stop_not_confirmed: 'Stop not confirmed',
};

// A chat with Crew access whose Crew connection is down: the daemon refuses the turn with
// "Crew connection is disconnected; authenticate and connect in Crew"
// (`crates/biorouter/src/crew/mod.rs`, `transport`). It is not the model's failure, and "Model
// request failed" sent people to the model's settings (live QA round 2, Q2-08).
const CREW_DISCONNECTED = 'Crew connection is disconnected';

function isCrewDisconnected(error: ChatTurnErrorData): boolean {
  return (
    error.message.includes(CREW_DISCONNECTED) ||
    (error.technicalDetails ?? '').includes(CREW_DISCONNECTED)
  );
}

function isBackendUnreachable(error: ChatTurnErrorData): boolean {
  return (
    (error.scope === 'transport' || isConnectionError(error.message)) &&
    BACKEND_CODES.has(error.code)
  );
}

function isMidStreamDrop(error: ChatTurnErrorData): boolean {
  return error.scope === 'transport' && MIDSTREAM_CODES.has(error.code);
}

function userFacingMessage(error: ChatTurnErrorData): string {
  if (isCrewDisconnected(error)) {
    return "This chat's Crew connection is offline. Open Crew and connect, then retry.";
  }

  const decoded = providerMessage(error.message) ?? providerMessage(error.technicalDetails ?? '');
  if (decoded) return decoded;

  if (isBackendUnreachable(error)) {
    return "Biorouter's backend is restarting or unreachable. Your chat is safe. Retry in a moment, or check that the app is running.";
  }

  if (isMidStreamDrop(error)) {
    return 'The connection dropped before the response finished. Retry to continue where it left off.';
  }

  if (error.scope === 'transport' || isConnectionError(error.message)) {
    return 'Biorouter could not reach the model provider. Check your connection and provider settings, then try again.';
  }

  return (
    error.message
      .replace(/^(?:Stream|Submit) error:\s*/i, '')
      .replace(/^provider_failure:\s*/i, '')
      .trim() || 'The model request failed before it produced a response.'
  );
}

export function presentChatTurnError(error: ChatTurnErrorData): ChatTurnErrorPresentation {
  // A chat whose Crew access stopped: the daemon refused the turn before any model request, and
  // says why in a sentence (final acceptance D-1). Never "Model request failed", and never the
  // workspace's JSON envelope an older daemon passed on — not even under the details.
  const crewRefusal = STOP_TITLES[error.code] ? null : crewTurnRefusalOf(error);
  if (crewRefusal) return { title: crewRefusal.title, message: crewRefusal.message };

  // The repetition guard ended the turn. Nothing failed and nobody needs to
  // retry: the person is told, in one sentence, which step Biorouter stopped
  // and how often it had run. The guard's own reason is written for the agent
  // ("The user did NOT decline it…") and stays its tool result; an older
  // daemon sent it here, so it is translated rather than shown (live QA,
  // 2026-11-11). Never "Model request failed", and no Details.
  if (error.code === TOOL_LOOP_CODE) {
    return {
      title: REPETITION_STOP_TITLE,
      message: repetitionStopNotice(error.message) ?? REPETITION_STOP_FALLBACK,
    };
  }

  let title = 'Model request failed';
  if (STOP_TITLES[error.code]) {
    title = STOP_TITLES[error.code];
  } else if (isCrewDisconnected(error)) {
    title = 'Crew is offline';
  } else if (error.providerKind && PROVIDER_TITLES[error.providerKind]) {
    title = PROVIDER_TITLES[error.providerKind];
  } else if (error.message.includes('insufficient_quota')) {
    title = 'Model quota exceeded';
  } else if (isBackendUnreachable(error)) {
    title = 'Backend unreachable';
  } else if (isMidStreamDrop(error)) {
    title = 'Connection dropped';
  } else if (error.scope === 'transport' || isConnectionError(error.message)) {
    title = 'Model connection failed';
  } else if (error.scope === 'session') {
    title = 'Model could not be prepared';
  } else if (error.scope === 'internal') {
    title = 'Model turn ended unexpectedly';
  }

  const message = userFacingMessage(error);
  const details = error.technicalDetails?.trim();
  return {
    title,
    message,
    details: details && details !== message ? details : undefined,
  };
}

// The daemon's stored turn-ending notice (`stop_notice` in
// `crates/biorouter/src/agents/mistakes.rs`, and the compaction failure in
// `agents/agent.rs`): "Ran into this error: <ProviderError>\n\n<advice>". It is
// an ordinary assistant message, so the model reads it next turn and a reload
// shows it; drawn as prose it was a second look for the same failure the live
// error row draws (vision V2-07).
const STORED_NOTICE = /^Ran into this error( trying to compact)?: */;

// `ProviderError`'s Display prefixes (`crates/biorouter/src/providers/errors.rs`).
const STORED_TITLES: ReadonlyArray<readonly [RegExp, string]> = [
  [/^Authentication error:\s*/, PROVIDER_TITLES.auth],
  [/^Context length exceeded:\s*/, PROVIDER_TITLES.context_length],
  [/^Rate limit exceeded:\s*/, PROVIDER_TITLES.rate_limit],
  [/^Server error:\s*/, PROVIDER_TITLES.server],
  [/^(?:Request failed|Execution error|Usage data error|Unsupported operation):\s*/, ''],
];

/** One stop at the end, never two (the daemon's `end_sentence`). */
function withStop(text: string): string {
  return /[.!?]["')\]}]*$/.test(text) ? text : `${text}.`;
}

/**
 * A stored "Ran into this error" notice as the error row presents it, or null
 * for any other text. The title names the kind of failure, the message is the
 * reason in words followed by the daemon's advice, and a raw provider response
 * (`… Response: {"error": …}`) moves under Details with the full error.
 */
export function presentStoredTurnError(text: string): ChatTurnErrorPresentation | null {
  const notice = STORED_NOTICE.exec(text);
  if (!notice) return null;
  const compact = notice[1] !== undefined;
  const rest = text.slice(notice[0].length);
  const split = rest.lastIndexOf('\n\n');
  const errorText = (split === -1 ? rest : rest.slice(0, split)).trim();
  const advice = (split === -1 ? '' : rest.slice(split + 2)).trim();
  if (!errorText) return null;

  let title = compact ? 'Could not compact this chat' : 'Model request failed';
  let reason = errorText;
  for (const [prefix, kindTitle] of STORED_TITLES) {
    if (!prefix.test(errorText)) continue;
    if (!compact && kindTitle) title = kindTitle;
    const stripped = errorText.replace(prefix, '');
    // "Server error: 502." keeps its prefix: a bare status is not a reason.
    if (/[A-Za-z]{3,}/.test(stripped)) reason = stripped;
    break;
  }
  if (!compact && errorText.includes('insufficient_quota')) title = 'Model quota exceeded';
  // A content filter's block (`content_filter_blocked_message` in
  // `providers/formats/openai.rs`) is stored as "Request failed: …"; the live
  // row titles it by its kind, policy, and the stored row stands in for it.
  if (!compact && / content filter blocked this /.test(errorText)) title = PROVIDER_TITLES.policy;

  const body = /\s*\bResponse:\s*/.exec(reason);
  if (body && body.index > 0) reason = reason.slice(0, body.index).trim();
  reason = reason.charAt(0).toUpperCase() + reason.slice(1);
  const message = [withStop(reason), advice ? withStop(advice) : ''].filter(Boolean).join(' ');
  return { title, message, details: body ? errorText : undefined };
}

/**
 * The transcript row that already shows this error: the daemon's stored notice
 * ("Ran into this error: …"), anywhere in the turn that failed. Null when the
 * transcript does not show it.
 *
 * The whole turn is searched, back to the user row that opened it, because
 * rows can land after the notice: a decision monitor's marker did, and reading
 * only the last row drew one content-filter failure twice, the stored row and
 * then the live one (live QA, 2026-11-11).
 */
export function turnErrorNoticeMessage(
  error: ChatTurnErrorData,
  messages: Message[]
): Message | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role === 'user') {
      if (opensTurn(message)) return null;
      continue;
    }
    if (message.metadata?.userVisible === false) continue;
    if (
      message.content.some(
        (content) => content.type === 'text' && content.text.includes(error.message)
      )
    ) {
      return message;
    }
  }

  return null;
}

export function hasVisibleTurnErrorMessage(error: ChatTurnErrorData, messages: Message[]): boolean {
  return turnErrorNoticeMessage(error, messages) !== null;
}

/**
 * Retry for the stored notice of the failure the chat is showing right now.
 * When the transcript already shows the failure, the live row is not drawn, so
 * its Retry rides on the stored row instead (BaseChat provides it, keyed by
 * the notice's id). Null everywhere else: history, shared and read-only views
 * draw the notice without a button.
 */
export const StoredTurnErrorRetryContext = createContext<{
  messageId: string;
  onRetry: () => void;
} | null>(null);

/** Whether the person may retry this failure from the chat. */
export function canRetryTurnError(error: ChatTurnErrorData): boolean {
  // A turn refused for its Crew access would be refused again: the way on is the chat's Crew bar.
  return error.retryable && !crewTurnRefusalOf(error);
}

/**
 * Where the chat draws its live turn error, so one failure draws once: as its
 * own row at the end of the transcript (`row`), or, when a stored notice in
 * the failed turn already shows it, not again, with the live row's Retry moved
 * onto that notice (`retryOnNoticeId`, null when the failure is not one to
 * retry).
 */
export function placeTurnError(
  error: ChatTurnErrorData | null | undefined,
  messages: Message[]
): { row: ChatTurnErrorData | null; retryOnNoticeId: string | null } {
  if (!error) return { row: null, retryOnNoticeId: null };
  const notice = turnErrorNoticeMessage(error, messages);
  if (!notice) return { row: error, retryOnNoticeId: null };
  return { row: null, retryOnNoticeId: notice.id && canRetryTurnError(error) ? notice.id : null };
}

export function ChatTurnError({
  error,
  onRetry,
}: {
  error: ChatTurnErrorData;
  /** Re-run the failed turn. Rendered as a "Retry" action when the error is retryable. */
  onRetry?: () => void;
}) {
  const presentation = presentChatTurnError(error);
  // A repetition stop is a notice, not a failure: one quiet transcript line in
  // muted ink, announced politely, with no danger glyph and no Retry (the same
  // call would be stopped again). The line is the sentence itself, as a
  // confirmed Stop's line is (`ChatTurnStopped`).
  if (error.code === TOOL_LOOP_CODE) {
    return (
      <div data-testid="chat-turn-loop-stop" className="mt-6 w-full">
        <div role="status">
          <TranscriptRow icon={Info} glyph="loop-stop" label={presentation.message} />
        </div>
      </div>
    );
  }
  const canRetry = !!onRetry && canRetryTurnError(error);

  // It sits a turn's width (24px) under the chat, as the agent's answer would
  // have.
  return (
    <TurnErrorRow
      presentation={presentation}
      alert
      className="mt-6"
      testId="chat-turn-error"
      onRetry={canRetry ? onRetry : undefined}
    />
  );
}

/**
 * The one error row (spec 3.6 "Errors"), for the live turn error and for the
 * daemon's stored notice alike: a transcript row, not a toast pasted into the
 * chat. A 16px warning glyph in danger ink, the title in `text-label`, the
 * message in muted `text-secondary`, Retry as the one secondary button, and the
 * raw provider payload behind the shared Disclosure.
 *
 * The message shows its first sentence, the error itself; advice after it
 * ("Your chat is safe. Retry in a moment…") is the info icon beside it, and a
 * longer tail joins the raw payload under Details (owner, 2026-11-13: no solid
 * chunk of text, every word one hover or one click away). A message that fits
 * one line stays whole.
 *
 * `alert` announces it: only for a failure that happens while the person
 * watches, never for one read back from history.
 */
export function TurnErrorRow({
  presentation,
  alert = false,
  className,
  testId,
  onRetry,
}: {
  presentation: ChatTurnErrorPresentation;
  alert?: boolean;
  className?: string;
  testId?: string;
  onRetry?: () => void;
}) {
  const { lead, tip, details: moreText } = foldForHelp(presentation.message);
  return (
    <div
      data-testid={testId}
      role={alert ? 'alert' : undefined}
      className={cn('flex w-full items-start gap-2', className)}
    >
      <AlertTriangle aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-text-danger" />
      <div className="min-w-0 flex-1">
        <div className="text-label text-text-default">{presentation.title}</div>
        <div className="text-secondary text-text-muted break-words">
          {lead}
          {tip ? (
            <>
              {' '}
              <InfoTip label={ERROR_TIP_NAME} help={tip} />
            </>
          ) : null}
        </div>
        {(moreText || presentation.details) && (
          <Disclosure label={transcriptCopy.turn.details} className="mt-1">
            <div className="flex flex-col gap-2">
              {moreText ? (
                <p className="text-secondary text-text-muted break-words">{moreText}</p>
              ) : null}
              {/* One well, as a tool call's expanded body is: the raw payload is
                  machine text, and the fill keeps it apart from the rows below. */}
              {presentation.details ? (
                <div className="whitespace-pre-wrap break-words rounded-element bg-background-well p-3 font-mono text-code text-text-muted [overflow-wrap:anywhere]">
                  {presentation.details}
                </div>
              ) : null}
            </div>
          </Disclosure>
        )}
      </div>
      {onRetry && (
        <Button
          type="button"
          variant="secondary"
          size="sm"
          data-testid="chat-turn-error-retry"
          onClick={onRetry}
          className="shrink-0"
        >
          {transcriptCopy.turn.retry}
        </Button>
      )}
    </div>
  );
}
