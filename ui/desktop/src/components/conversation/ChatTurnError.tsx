import { AlertTriangle } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Disclosure } from '../ui/disclosure';
import { transcriptCopy } from '../transcript/copy';
import type { Message } from '../../api';
import type { ChatTurnErrorData } from '../../types/turnError';
import { isConnectionError } from '../../utils/conversionUtils';
import { crewTurnRefusalOf } from '../crew/access/crewTurnRefusal';

interface ChatTurnErrorPresentation {
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

export function hasVisibleTurnErrorMessage(error: ChatTurnErrorData, messages: Message[]): boolean {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role !== 'assistant' || message.metadata?.userVisible === false) continue;

    return message.content.some(
      (content) => content.type === 'text' && content.text.includes(error.message)
    );
  }

  return false;
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
  // A turn refused for its Crew access would be refused again: the way on is the chat's Crew bar.
  const canRetry = error.retryable && !!onRetry && !crewTurnRefusalOf(error);

  // A transcript row, not a toast pasted into the chat (spec 3.6 "Errors"): a
  // 16px warning glyph in danger ink, the title in `text-label`, the message in
  // muted `text-secondary`, Retry as the one secondary button, and the raw
  // provider payload behind the shared Disclosure. It sits a turn's width (24px)
  // under the chat, as the agent's answer would have.
  return (
    <div data-testid="chat-turn-error" role="alert" className="mt-6 flex w-full items-start gap-2">
      <AlertTriangle aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-text-danger" />
      <div className="min-w-0 flex-1">
        <div className="text-label text-text-default">{presentation.title}</div>
        <div className="text-secondary text-text-muted break-words">{presentation.message}</div>
        {presentation.details && (
          <Disclosure label={transcriptCopy.turn.details} className="mt-1">
            <div className="whitespace-pre-wrap break-words font-mono text-code text-text-muted [overflow-wrap:anywhere]">
              {presentation.details}
            </div>
          </Disclosure>
        )}
      </div>
      {canRetry && (
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
