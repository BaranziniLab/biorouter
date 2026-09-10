import { NotificationSurface } from '../alerts/NotificationSurface';
import type { Message } from '../../api';
import type { ChatTurnErrorData } from '../../types/turnError';
import { isConnectionError } from '../../utils/conversionUtils';

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
  let title = 'Model request failed';
  if (STOP_TITLES[error.code]) {
    title = STOP_TITLES[error.code];
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
  const canRetry = error.retryable && !!onRetry;

  return (
    <div data-testid="chat-turn-error" className="mt-4">
      <NotificationSurface
        status="error"
        role="alert"
        title={presentation.title}
        message={presentation.message}
        actions={
          canRetry ? (
            <button
              type="button"
              data-testid="chat-turn-error-retry"
              onClick={onRetry}
              className="inline-flex items-center rounded-md border border-border-subtle bg-background-default px-2.5 py-1 text-[13px] font-medium text-text-default transition-colors hover:bg-background-medium"
            >
              Retry
            </button>
          ) : undefined
        }
      >
        {presentation.details && (
          <details className="mt-2.5 text-xs text-text-muted">
            <summary className="w-fit cursor-pointer select-none font-medium text-text-default">
              Technical details
            </summary>
            <div className="mt-2 whitespace-pre-wrap break-words font-mono leading-relaxed [overflow-wrap:anywhere]">
              {presentation.details}
            </div>
          </details>
        )}
      </NotificationSurface>
    </div>
  );
}
