import { useMemo, useState } from 'react';
import type { ActionRequired, SecretKeyRequest } from '../api';
import { submitSecrets } from '../api';
import { Button } from './ui/button';
import { SecretInput } from './ui/secret-input';
import { InfoTip } from './ui/info-tip';
import { Check, Lock, X } from './icons/app-icons';
import { TranscriptRow } from './TranscriptRow';
import { SECRET_COPY } from './toolCallCopy';
import { userActionHeaders } from '../utils/userAction';

/**
 * Issue #117. The one surface a credential is typed into.
 *
 * ⚠ **This card does NOT answer through the conversation.** Every other
 * `ActionRequired` card resolves by appending a message — `ElicitationRequest`
 * builds an `elicitationResponse` whose `user_data` is marked `agentVisible`,
 * and the agent forwards that whole object to the waiting request. Doing the
 * same here with `type="password"` inputs would hide the characters from the
 * person typing them and from nobody else: the value would still be serialised
 * into the transcript, persisted to the session row, and replayed into the next
 * prompt.
 *
 * So the values go straight to `POST /action-required/secrets`, which writes
 * them to the OS credential store and releases the parked install with the key
 * NAMES. No message is created, `append` is never called, and there is no
 * `SecretResponse` content type for one to be created with.
 *
 * The request carries `userActionHeaders()` — DR-16's proof that this came from
 * the person at the keyboard. The model reaches the same daemon over the same
 * HTTP with the same secret key; without the proof it could satisfy its own
 * credential card and drive the install past the step that exists to involve a
 * person.
 */
interface Props {
  isCancelledMessage: boolean;
  actionRequiredContent: ActionRequired & { type: 'actionRequired' };
}

type Status =
  | { kind: 'editing'; missing?: string[]; error?: string }
  | { kind: 'saving' }
  | { kind: 'configured'; keys: string[] }
  | { kind: 'cancelled' }
  | { kind: 'gone' };

export default function SecretRequestCard({ isCancelledMessage, actionRequiredContent }: Props) {
  const data = actionRequiredContent.data;
  const isSecretRequest = data.actionType === 'secretRequest';

  const [values, setValues] = useState<Record<string, string>>({});
  const [status, setStatus] = useState<Status>({ kind: 'editing' });

  const keys: SecretKeyRequest[] = useMemo(
    () => (isSecretRequest ? data.keys : []),
    [isSecretRequest, data]
  );
  const required = keys.filter((k) => k.required);
  const optional = keys.filter((k) => !k.required);
  const missingRequired = required.some((k) => !(values[k.key] ?? '').trim());

  if (!isSecretRequest) return null;
  const { id, prompt, destination } = data;

  const extensionName = destination.kind === 'extensionEnv' ? destination.extensionName : null;

  const post = async (body: { cancelled: true } | { values: Record<string, string> }) => {
    setStatus({ kind: 'saving' });
    try {
      const response = await submitSecrets({
        body: { id, ...body },
        headers: await userActionHeaders(),
      });
      // ⚠ The response is read for STATUS only. It carries `configuredKeys` and
      // `missing` — names — and the daemon has nowhere in its shape to put a
      // value back. Never widen this to echo one.
      const result = (response.data ?? {}) as {
        status?: string;
        configuredKeys?: string[];
        missing?: string[];
        reason?: string;
      };
      switch (result.status) {
        case 'configured':
          setStatus({ kind: 'configured', keys: result.configuredKeys ?? [] });
          return;
        case 'cancelled':
          setStatus({ kind: 'cancelled' });
          return;
        case 'incomplete':
          setStatus({ kind: 'editing', missing: result.missing ?? [] });
          return;
        case 'unknown':
          // The install stopped waiting — the turn ended, or another window
          // answered first. Saying so beats a spinner that never resolves.
          setStatus({ kind: 'gone' });
          return;
        default:
          setStatus({
            kind: 'editing',
            error:
              result.reason ??
              ((response.error as { error?: string } | undefined)?.error ||
                SECRET_COPY.storeFailed),
          });
      }
    } catch (error) {
      setStatus({
        kind: 'editing',
        error: error instanceof Error ? error.message : SECRET_COPY.storeFailed,
      });
    }
  };

  // Resolved states are transcript rows: the card has done its job.
  if (isCancelledMessage || status.kind === 'cancelled') {
    return (
      <div className="biorouter-message-content text-body">
        <TranscriptRow icon={X} label={SECRET_COPY.canceled} />
      </div>
    );
  }

  if (status.kind === 'gone') {
    return (
      <div className="biorouter-message-content text-body">
        <TranscriptRow icon={X} label={SECRET_COPY.gone(extensionName)} />
      </div>
    );
  }

  if (status.kind === 'configured') {
    // Names, never values: this line is part of the transcript.
    return (
      <div className="biorouter-message-content text-body">
        <TranscriptRow icon={Check} label={SECRET_COPY.configured(extensionName, status.keys)} />
      </div>
    );
  }

  const busy = status.kind === 'saving';
  const missing = status.kind === 'editing' ? (status.missing ?? []) : [];

  const field = (entry: SecretKeyRequest) => {
    const flagged = missing.includes(entry.key);
    const inputId = `secret-${id}-${entry.key}`;
    const helpId = `${inputId}-help`.replace(/[^a-zA-Z0-9_-]/g, '_');
    return (
      <div key={entry.key} className="flex min-w-0 flex-col gap-1.5">
        <div className="flex min-w-0 items-center gap-1">
          <label htmlFor={inputId} className="truncate text-label text-text-default">
            {entry.label}
            {!entry.required && <span className="text-text-muted"> {SECRET_COPY.optional}</span>}
          </label>
          {/* The description is help, not a second copy of the placeholder: it
              lives in an InfoTip beside the label (never inside it) and the
              field points at the same text. */}
          {entry.description && (
            <InfoTip id={helpId} label={entry.label} help={entry.description} />
          )}
        </div>
        <SecretInput
          id={inputId}
          revealLabel={entry.label}
          // Masked by default with an intentional reveal, and NEVER pre-filled:
          // a default value here would have to be read back out of the
          // credential store, which is the one thing this whole path exists to
          // avoid.
          className={flagged ? '!border-border-danger' : undefined}
          aria-invalid={flagged || undefined}
          aria-describedby={entry.description ? helpId : undefined}
          value={values[entry.key] ?? ''}
          onChange={(e) => setValues((prev) => ({ ...prev, [entry.key]: e.target.value }))}
          disabled={busy}
        />
      </div>
    );
  };

  // Asking: the one card recipe. Required fields first, then the optional ones,
  // each marked "(optional)" rather than filed under a second caps heading.
  return (
    <div className="biorouter-message-content br-enter text-body flex min-w-0 flex-col gap-3 rounded-container border border-border-subtle bg-background-default p-4">
      <div className="flex min-w-0 flex-col gap-1">
        <div className="text-label text-text-default">{prompt || SECRET_COPY.fallbackPrompt}</div>
        {/* The decision point, so the reassurance stays visible in one short
            line; the full sentence is in the InfoTip. */}
        <div className="flex items-center gap-1.5 text-supporting text-text-muted">
          <Lock aria-hidden="true" className="size-3.5 shrink-0" />
          <span>{SECRET_COPY.reassurance}</span>
          <InfoTip label={SECRET_COPY.reassuranceName} help={SECRET_COPY.reassuranceHelp} />
        </div>
      </div>

      {required.map(field)}
      {optional.map(field)}

      {status.kind === 'editing' && status.error && (
        <p role="alert" className="text-supporting text-text-danger">
          {status.error}
        </p>
      )}
      {missing.length > 0 && (
        <p className="text-supporting text-text-danger">{SECRET_COPY.stillNeeded(missing)}</p>
      )}

      <div className="flex items-center justify-end gap-2">
        <Button variant="ghost" size="sm" disabled={busy} onClick={() => post({ cancelled: true })}>
          {SECRET_COPY.cancel}
        </Button>
        <Button size="sm" disabled={busy || missingRequired} onClick={() => post({ values })}>
          {busy ? SECRET_COPY.saving : SECRET_COPY.save}
        </Button>
      </div>
    </div>
  );
}
