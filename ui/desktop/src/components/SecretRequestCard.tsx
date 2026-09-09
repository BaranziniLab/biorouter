import { useMemo, useState } from 'react';
import type { ActionRequired, SecretKeyRequest } from '../api';
import { submitSecrets } from '../api';
import { Button } from './ui/button';
import { Check, Lock } from './icons/app-icons';
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
  const [revealed, setRevealed] = useState<Record<string, boolean>>({});
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
                'Biorouter could not store these values.'),
          });
      }
    } catch (error) {
      setStatus({
        kind: 'editing',
        error: error instanceof Error ? error.message : 'Biorouter could not store these values.',
      });
    }
  };

  if (isCancelledMessage || status.kind === 'cancelled') {
    return (
      <div className="biorouter-message-content bg-background-muted rounded-2xl px-4 py-2 text-body text-text-default">
        Credential setup was canceled. Nothing was installed.
      </div>
    );
  }

  if (status.kind === 'gone') {
    return (
      <div className="biorouter-message-content bg-background-muted rounded-2xl px-4 py-2 text-body text-text-default">
        This request is no longer waiting for an answer. Ask again to configure{' '}
        {extensionName ?? 'the extension'}.
      </div>
    );
  }

  if (status.kind === 'configured') {
    return (
      <div className="biorouter-message-content bg-background-muted rounded-2xl px-4 py-2 text-body text-text-default">
        <div className="flex items-center gap-2">
          <Check className="w-5 h-5 text-text-muted" />
          {/* Names, never values — this line is part of the transcript. */}
          <span>
            Credentials configured
            {extensionName ? ` for ${extensionName}` : ''}
            {status.keys.length > 0 ? `: ${status.keys.join(', ')}` : ''}
          </span>
        </div>
      </div>
    );
  }

  const busy = status.kind === 'saving';
  const missing = status.kind === 'editing' ? (status.missing ?? []) : [];

  const field = (entry: SecretKeyRequest) => {
    const isRevealed = revealed[entry.key] ?? false;
    const flagged = missing.includes(entry.key);
    return (
      <div key={entry.key}>
        <div className="flex items-center justify-between gap-2">
          <label htmlFor={`secret-${id}-${entry.key}`} className="block text-xs font-semibold mb-1">
            {entry.label}
            {entry.required && <span className="text-text-danger"> *</span>}
          </label>
          <button
            type="button"
            className="text-[11px] text-text-muted underline"
            onClick={() => setRevealed((prev) => ({ ...prev, [entry.key]: !isRevealed }))}
          >
            {isRevealed ? 'Hide' : 'Show'}
          </button>
        </div>
        <input
          id={`secret-${id}-${entry.key}`}
          // Masked by default with an intentional reveal, and NEVER pre-filled:
          // a default value here would have to be read back out of the
          // credential store, which is the one thing this whole path exists to
          // avoid.
          type={isRevealed ? 'text' : 'password'}
          className={[
            'biorouter-modal-panel w-full rounded-md px-3 py-2 text-sm',
            flagged ? '!border-border-danger' : '',
          ].join(' ')}
          placeholder={entry.description ?? ''}
          value={values[entry.key] ?? ''}
          onChange={(e) => setValues((prev) => ({ ...prev, [entry.key]: e.target.value }))}
          autoComplete="off"
          spellCheck={false}
          disabled={busy}
        />
        {entry.description && (
          <p className="text-[11px] text-text-muted mt-1 leading-relaxed">{entry.description}</p>
        )}
      </div>
    );
  };

  return (
    <div className="flex flex-col">
      <div className="biorouter-message-content bg-background-muted rounded-2xl rounded-b-none px-4 py-2 text-body text-text-default">
        <div className="flex items-center gap-2">
          <Lock className="w-4 h-4 text-text-muted shrink-0" />
          <span>{prompt || 'Biorouter needs some credentials.'}</span>
        </div>
      </div>
      <div className="biorouter-message-content bg-background-default border border-border-subtle rounded-b-2xl px-4 py-3 text-body space-y-3">
        <p className="text-[11px] text-text-muted leading-relaxed">
          These go straight to this machine's credential store. They are not added to the
          conversation and the model never sees them.
        </p>

        {required.length > 0 && (
          <div className="space-y-3">
            <p className="text-xs font-semibold text-text-default uppercase tracking-wide">
              Required
            </p>
            {required.map(field)}
          </div>
        )}

        {optional.length > 0 && (
          <div className="space-y-3">
            <p className="text-xs font-semibold text-text-muted uppercase tracking-wide">
              Optional
            </p>
            {optional.map(field)}
          </div>
        )}

        {status.kind === 'editing' && status.error && (
          <p className="text-xs text-text-danger">{status.error}</p>
        )}
        {missing.length > 0 && (
          <p className="text-xs text-text-danger">Still needed: {missing.join(', ')}</p>
        )}

        <div className="flex items-center justify-end gap-2 pt-1">
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => post({ cancelled: true })}
          >
            Cancel
          </Button>
          <Button size="sm" disabled={busy || missingRequired} onClick={() => post({ values })}>
            {busy ? 'Saving…' : 'Save and continue'}
          </Button>
        </div>
      </div>
    </div>
  );
}
