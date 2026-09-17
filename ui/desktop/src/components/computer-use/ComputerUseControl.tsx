import { useCallback, useEffect, useRef, useState } from 'react';
import { Monitor } from '../icons/app-icons';
import { ComputerUseRuntimeDetails } from './ComputerUseSetup';
import { Button } from '../ui/button';
import { isBrowserSurface } from '../../utils/surface';
import {
  computerUseDecision,
  computerUseSetup,
  computerUseStatus,
  type ComputerUseStatus,
} from './computerUseApi';

function requestError(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  if (error && typeof error === 'object') {
    if ('message' in error && typeof error.message === 'string') return error.message;
    if ('error' in error && typeof error.error === 'string') return error.error;
  }
  return 'Computer Use could not confirm this change. Try again.';
}

export function ComputerUseControl({ sessionId }: { sessionId: string }) {
  return <SessionComputerUseControl key={sessionId} sessionId={sessionId} />;
}

function SessionComputerUseControl({ sessionId }: { sessionId: string }) {
  const [status, setStatus] = useState<ComputerUseStatus>();
  const [expanded, setExpanded] = useState(false);
  const [approvalKey, setApprovalKey] = useState('');
  const [error, setError] = useState('');
  const [loadError, setLoadError] = useState('');
  const [sending, setSending] = useState(false);
  const [checking, setChecking] = useState(false);
  const mutation = useRef(false);
  const mounted = useRef(true);
  const generation = useRef(0);
  const polling = useRef(false);
  const browser = isBrowserSurface();

  const refresh = useCallback(async () => {
    if (polling.current || mutation.current) return;
    polling.current = true;
    const startedAt = generation.current;
    try {
      const next = await computerUseStatus(sessionId);
      if (
        mounted.current &&
        startedAt === generation.current &&
        !mutation.current &&
        next.session_id === sessionId
      ) {
        setStatus(next);
        setLoadError('');
      }
    } catch (failure) {
      if (mounted.current && startedAt === generation.current) setLoadError(requestError(failure));
    } finally {
      polling.current = false;
    }
  }, [sessionId]);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    const timer = window.setInterval(() => void refresh(), 2000);
    return () => {
      mounted.current = false;
      window.clearInterval(timer);
    };
  }, [refresh]);

  const decide = async (action: 'consent' | 'revoke') => {
    if (!status || mutation.current || (action === 'consent' && !status.requested)) return;
    generation.current += 1;
    mutation.current = true;
    setSending(true);
    setError('');
    try {
      const next = await computerUseDecision(status, action, approvalKey);
      if (mounted.current) {
        setStatus(next);
        setExpanded(false);
        setApprovalKey('');
      }
    } catch (failure) {
      if (mounted.current) setError(requestError(failure));
    } finally {
      mutation.current = false;
      if (mounted.current) setSending(false);
    }
  };

  const checkSetup = async () => {
    setChecking(true);
    setError('');
    try {
      const runtime = await computerUseSetup();
      if (mounted.current) setStatus((current) => current && { ...current, runtime });
    } catch (failure) {
      if (mounted.current) setError(requestError(failure));
    } finally {
      if (mounted.current) setChecking(false);
    }
  };

  if (!status)
    return loadError ? (
      <div className="mx-3 mb-2 text-supporting text-text-muted">
        <p role="alert">Computer Use status unavailable: {loadError}</p>
        <Button size="sm" variant="ghost" onClick={() => void refresh()}>
          Retry
        </Button>
      </div>
    ) : null;
  if (status.enabled === false && status.state !== 'active') return null;
  const active = status.state === 'active';
  const busy = status.state === 'busy';
  const requested = status.requested && !active;
  const detailsVisible = expanded || requested;
  const sharing = status.public_model !== false;

  return (
    <section aria-label="Computer Use" className="mx-3 mb-2 text-supporting text-text-default">
      <div className="flex min-w-0 items-center gap-2">
        <Monitor className="size-4 shrink-0" aria-hidden="true" />
        <span className="min-w-0 flex-1 break-words" role="status">
          {active
            ? 'Computer use active'
            : busy
              ? 'Computer use busy'
              : status.state === 'stopped'
                ? 'Computer use stopped'
                : 'Computer Use'}
          {active && <span className="text-text-muted"> · {status.target}</span>}
        </span>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => setExpanded(!expanded)}
          aria-expanded={Boolean(detailsVisible)}
        >
          {active || requested ? 'Details' : 'Set up'}
        </Button>
        {active && (
          <Button
            size="sm"
            variant="destructive"
            disabled={sending}
            onClick={() => void decide('revoke')}
          >
            {sending ? 'Stopping…' : 'Stop'}
          </Button>
        )}
      </div>
      {detailsVisible && (
        <div className="mt-2 space-y-2">
          <p className="break-words">
            Computer: <strong>{status.target}</strong>
          </p>
          {browser && (
            <p className="text-text-muted">
              This controls the computer running Biorouter, which may be different from the computer
              displaying this browser.
            </p>
          )}
          <p className="break-words">
            Model: {status.model} · {status.provider}
          </p>
          <p className="break-words">Data destination: {status.destination}</p>
          {requested && !busy ? (
            <>
              <p className="whitespace-pre-line">
                {status.disclosure ||
                  (sharing
                    ? 'Allow computer use for this request? Screenshots, app text, and open-window information may be sent to the provider above, including sensitive information. Biorouter can type, click, use the cursor and change focus, and make changes until its reply finishes or you stop it.'
                    : 'Allow Biorouter to view and control this computer for this request? It can read app content, type, click, use the cursor and change focus, and make changes until its reply finishes or you stop it. Private classification does not mean processing happens on this computer.')}
              </p>
              {status.handoff_required && !status.disclosure && (
                <p className="text-text-warning">
                  Content left open by another task may be visible. Close or hide anything you do
                  not want shared before continuing.
                </p>
              )}
              <p className="text-text-muted">
                Stop prevents further actions; it cannot undo changes already delivered to an app.
                Desktop apps and files remain shared with other tasks.
              </p>
            </>
          ) : busy ? (
            <p>Another task is using this desktop. Stop that task before starting here.</p>
          ) : !active ? (
            <p>
              Ask Biorouter to use the computer. Approval appears before it starts and lasts through
              that request.
            </p>
          ) : null}
          {status.runtime && <ComputerUseRuntimeDetails runtime={status.runtime} />}
          <Button size="sm" variant="outline" disabled={checking} onClick={() => void checkSetup()}>
            {checking ? 'Checking…' : 'Check OS permissions'}
          </Button>
          {requested && !busy && (
            <>
              {browser && (
                <label className="block">
                  Computer Use approval key
                  <input
                    type="password"
                    autoComplete="off"
                    spellCheck={false}
                    value={approvalKey}
                    onChange={(event) => setApprovalKey(event.target.value)}
                    className="mt-1 block w-full rounded-md border border-border-default bg-background-default px-2 py-1"
                  />
                  <span className="mt-1 block text-text-muted">
                    Enter the passphrase you chose when starting biorouter serve
                    --computer-use-approval. It stays only in this chat view and is cleared after
                    approval.
                  </span>
                </label>
              )}
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  disabled={sending || (browser && !approvalKey.trim())}
                  onClick={() => void decide('consent')}
                >
                  {sending
                    ? 'Allowing…'
                    : sharing
                      ? 'Allow control and sharing'
                      : 'Allow for this task'}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={sending}
                  onClick={() => void decide('revoke')}
                >
                  Cancel
                </Button>
              </div>
            </>
          )}
        </div>
      )}
      {loadError && (
        <p className="mt-2 text-text-muted">
          Status refresh failed. Stop remains available while control is active.
        </p>
      )}
      {error && (
        <p role="alert" className="mt-2 text-text-danger">
          {error}
        </p>
      )}
    </section>
  );
}
