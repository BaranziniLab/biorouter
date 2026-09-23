import { useCallback, useEffect, useRef, useState } from 'react';
import { ChevronDown, ChevronUp, Monitor, X } from '../icons/app-icons';
import { CopilotRuntimeDetails, runtimeVerdict } from './CopilotSetup';
import { PermissionCheckButton } from './PermissionCheckButton';
import { Button } from '../ui/button';
import { isBrowserSurface } from '../../utils/surface';
import {
  CopilotNotApplicable,
  copilotDecision,
  copilotSetup,
  copilotStatus,
  type CopilotStatus,
} from './copilotApi';

/**
 * The panel's own surface. It sits on the composer bar's `--background-canvas`
 * ground, so it takes one surface step up plus a hairline -- the same neutral
 * recipe `ui/note.tsx` uses, which the sibling `PinnedModelNote` on the rails
 * above it already paints. `--radius-container` is the ladder rung for a panel;
 * `--radius-element` would make it read as a control.
 *
 * Every class here has existing call sites, so none is a freshly written utility
 * that can silently fail to generate under `BIOROUTER_NO_HMR`.
 */
const PANEL_SHELL =
  'mx-3 mb-2 rounded-container border border-border-subtle bg-background-muted px-3 py-2.5 text-supporting';

const dismissedActivities = new Map<string, string>();

function rememberDismissal(sessionId: string, activity: string) {
  dismissedActivities.delete(sessionId);
  dismissedActivities.set(sessionId, activity);
  if (dismissedActivities.size > 100) {
    const oldest = dismissedActivities.keys().next().value;
    if (oldest !== undefined) dismissedActivities.delete(oldest);
  }
}

function requestError(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  if (error && typeof error === 'object') {
    if ('message' in error && typeof error.message === 'string') return error.message;
    if ('error' in error && typeof error.error === 'string') return error.error;
  }
  return 'Biorouter Copilot could not confirm this change. Try again.';
}

export function CopilotControl({ sessionId }: { sessionId: string }) {
  return <SessionCopilotControl key={sessionId} sessionId={sessionId} />;
}

function SessionCopilotControl({ sessionId }: { sessionId: string }) {
  const [status, setStatus] = useState<CopilotStatus>();
  const [expanded, setExpanded] = useState(false);
  const [dismissedActivity, setDismissedActivity] = useState(() =>
    dismissedActivities.get(sessionId)
  );
  const observedActivity = useRef(false);
  const [approvalKey, setApprovalKey] = useState('');
  const [error, setError] = useState('');
  const [loadError, setLoadError] = useState('');
  // A refusal this chat's mode produces, which re-asking cannot change.
  const [notApplicable, setNotApplicable] = useState(false);
  const [sending, setSending] = useState(false);
  const [checking, setChecking] = useState(false);
  // Only that a check completed. The SENTENCE is derived from the runtime
  // currently on screen, so a polled change can never contradict it.
  const [checked, setChecked] = useState(false);
  const mutation = useRef(false);
  // A permission probe must NEVER take `mutation`: that flag is what `decide`
  // checks, so sharing it makes Allow and Stop silent no-ops for the probe's
  // duration -- and Stop is the safety control of a desktop-control feature.
  // Its own flag only widens the POLL guard.
  const probing = useRef(false);
  const mounted = useRef(true);
  const generation = useRef(0);
  const polling = useRef(false);
  const browser = isBrowserSurface();

  const refresh = useCallback(async () => {
    if (polling.current || mutation.current || probing.current) return;
    polling.current = true;
    const startedAt = generation.current;
    try {
      const next = await copilotStatus(sessionId);
      if (
        mounted.current &&
        startedAt === generation.current &&
        !mutation.current &&
        next.session_id === sessionId
      ) {
        if (next.requested || next.state === 'active') observedActivity.current = true;
        setStatus(next);
        setLoadError('');
      }
    } catch (failure) {
      if (!mounted.current || startedAt !== generation.current) return;
      // A refusal the chat's own mode produces is permanent. Re-asking cannot
      // change it, so record it as "not applicable" rather than as an error:
      // the panel disappears and the poll below stops. Treating it as retryable
      // left a dead alert above the composer AND kept the 2 s poll running,
      // which re-spawns the native helper's PowerShell/UIA bridge every 30 s
      // for a chat that can never use it.
      if (failure instanceof CopilotNotApplicable) setNotApplicable(true);
      else setLoadError(requestError(failure));
    } finally {
      polling.current = false;
    }
  }, [sessionId]);

  useEffect(() => {
    mounted.current = true;
    // Nothing left to poll for once the answer is settled. Re-running this
    // effect on `notApplicable` is what tears the existing interval down; the
    // cleanup is returned on BOTH paths so unmount still marks us unmounted,
    // otherwise an in-flight refresh could set state on a dead component.
    const timer = notApplicable ? undefined : window.setInterval(() => void refresh(), 2000);
    if (!notApplicable) void refresh();
    return () => {
      mounted.current = false;
      if (timer !== undefined) window.clearInterval(timer);
    };
  }, [refresh, notApplicable]);

  const decide = async (action: 'consent' | 'revoke') => {
    if (!status || mutation.current || (action === 'consent' && !status.requested)) return;
    generation.current += 1;
    mutation.current = true;
    setSending(true);
    setError('');
    try {
      const next = await copilotDecision(status, action, approvalKey);
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
    // Claim this generation so an already in-flight poll cannot overwrite the
    // freshly probed runtime with the pre-refresh cached one. `probing` holds
    // off the NEXT poll; `mutation` is deliberately untouched, so a decision
    // stays possible while a check is in flight.
    generation.current += 1;
    probing.current = true;
    try {
      const runtime = await copilotSetup();
      if (mounted.current) {
        setStatus((current) => current && { ...current, runtime });
        setChecked(true);
      }
    } catch (failure) {
      if (mounted.current) {
        setError(requestError(failure));
        setChecked(false);
      }
    } finally {
      probing.current = false;
      if (mounted.current) setChecking(false);
    }
  };

  // A chat whose mode forbids Biorouter Copilot has no panel at all -- not an empty
  // one, and certainly not an error one. This sits ABOVE the `!status` branch
  // because `status` stays undefined when the very first read is refused.
  if (notApplicable) return null;
  if (!status) return null;
  // Older backends mark every completed reply stopped. Only an observed request
  // or the new session-owned activity identifier proves this chat used Copilot.
  if (
    !status.activity_id &&
    !status.requested &&
    status.state !== 'active' &&
    !observedActivity.current
  )
    return null;
  if (status.enabled === false && status.state !== 'active') return null;
  const active = status.state === 'active';
  const busy = status.state === 'busy';
  const requested = status.requested && !active;
  const detailsVisible = expanded || requested;
  const sharing = status.public_model !== false;
  const detailsId = `copilot-details-${sessionId}`;
  // A pending approval forces the panel open, because Allow and Cancel live
  // inside it. Offering a control that cannot close it would be a lie, so the
  // disclosure is withheld for exactly that state and returns once decided.
  const collapsible = !requested;
  const activityId = status.activity_id ?? (active || requested ? status.challenge_id : 'observed');
  const activity = `${activityId}:${status.state}:${status.requested}`;
  const dismissed = dismissedActivity === activity;
  if (dismissed) {
    if (!active && !requested) return null;
    return (
      <div className="mx-3 mb-2 flex flex-wrap items-center gap-2 text-supporting">
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            dismissedActivities.delete(sessionId);
            setDismissedActivity(undefined);
          }}
        >
          {active ? 'Show active Biorouter Copilot' : 'Show Biorouter Copilot request'}
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
        {error && (
          <p role="alert" className="text-text-danger">
            {error}
          </p>
        )}
      </div>
    );
  }

  return (
    <section aria-label="Biorouter Copilot" className={`${PANEL_SHELL} text-text-default`}>
      <div className="flex min-w-0 items-center gap-2">
        <Monitor className="size-4 shrink-0" aria-hidden="true" />
        <span className="min-w-0 flex-1 break-words" role="status">
          {active
            ? 'Biorouter Copilot active'
            : busy
              ? 'Biorouter Copilot busy'
              : status.state === 'stopped'
                ? 'Biorouter Copilot stopped'
                : 'Biorouter Copilot'}
          {active && <span className="text-text-muted"> · {status.target}</span>}
        </span>
        {collapsible && (
          <Button
            size="sm"
            shape="round"
            variant="ghost"
            onClick={() => setExpanded(!expanded)}
            aria-expanded={detailsVisible}
            aria-controls={detailsId}
            // The container below is always rendered and toggled with `hidden`,
            // so this IDREF always resolves. A control pointing at an element
            // that does not exist is what a collapsed-and-unmounted panel gives.
            aria-label={
              detailsVisible ? 'Hide Biorouter Copilot details' : 'Show Biorouter Copilot details'
            }
          >
            {detailsVisible ? (
              <ChevronDown
                className="h-icon-row w-icon-row shrink-0 text-text-muted"
                aria-hidden="true"
              />
            ) : (
              <ChevronUp
                className="h-icon-row w-icon-row shrink-0 text-text-muted"
                aria-hidden="true"
              />
            )}
          </Button>
        )}
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
        <Button
          size="sm"
          shape="round"
          variant="ghost"
          aria-label="Dismiss Biorouter Copilot banner"
          title={
            active
              ? 'Hide details; control remains active and Stop stays available'
              : 'Dismiss banner'
          }
          onClick={() => {
            rememberDismissal(sessionId, activity);
            setDismissedActivity(activity);
          }}
        >
          <X className="size-4" aria-hidden="true" />
        </Button>
      </div>
      <div id={detailsId} hidden={!detailsVisible} className="mt-2 space-y-2">
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
                  ? 'Allow Biorouter Copilot to view and control this computer for this request? Screenshots, app text, and open-window information may be sent to the provider above, including sensitive information. It can type, click, use the cursor and change focus, and make changes until its reply finishes or you stop it.'
                  : 'Allow Biorouter to view and control this computer for this request? It can read app content, type, click, use the cursor and change focus, and make changes until its reply finishes or you stop it. Private classification does not mean processing happens on this computer.')}
            </p>
            {status.handoff_required && !status.disclosure && (
              <p className="text-text-warning">
                Content left open by another task may be visible. Close or hide anything you do not
                want shared before continuing.
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
        {status.runtime && <CopilotRuntimeDetails runtime={status.runtime} />}
        <PermissionCheckButton
          label="Check OS permissions"
          checking={checking}
          checked={checked}
          verdict={status.runtime ? runtimeVerdict(status.runtime) : undefined}
          onCheck={() => void checkSetup()}
        />
        {requested && !busy && (
          <>
            {browser && (
              <label className="block">
                Biorouter Copilot approval key
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
