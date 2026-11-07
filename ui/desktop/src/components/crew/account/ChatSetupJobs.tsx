import { useCallback, useEffect, useRef, useState } from 'react';
import { Server } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { SignInDialog } from '../auth/SignInDialog';
import { sanitizeDisplayText } from '../identity';
import { listSetupJobs, setupActive, setupStartedHere, type SetupRun } from '../onboarding/setup';
import { onCrewChanged } from '../state/useCrewChanges';
import { setupJobsCopy } from './copy';

/** How often Crew asks the daemon for setups started elsewhere, while the window is visible. */
export const SETUP_JOBS_POLL_MS = 5000;

/** What a setup this window showed says once it failed (WI-15). */
export const setupFailedCopy = {
  failed: (server: string) => `Setting up Crew on ${server} stopped.`,
  dismiss: 'Dismiss',
  dismissLabel: (server: string) => `Dismiss the note about ${server}`,
} as const;

function visible(): boolean {
  return typeof document === 'undefined' || document.visibilityState !== 'hidden';
}

/** The server a job names, as the person reads it. */
function jobServer(run: SetupRun): string {
  return (
    sanitizeDisplayText(run.server?.login) ||
    sanitizeDisplayText(run.server?.user) ||
    setupJobsCopy.theServer
  );
}

export interface OutsideSetupJobs {
  /** The setups started elsewhere that are still running, or waiting for the person. */
  jobs: SetupRun[];
  /** The ones of those this window showed that failed since, until the person dismisses each. */
  failed: SetupRun[];
  dismiss(jobId: string): void;
}

/** Whether a change on Crew's change feed may be about a setup job. */
function aboutSetup(delta: { truncated: boolean; changes: { kind: string }[] }): boolean {
  return delta.truncated || delta.changes.some((change) => change.kind.startsWith('setup'));
}

/**
 * The setups this computer's daemon is running that this window did not start: a chat's
 * `crew__change setup` or `open`, or a terminal's `biorouter crew setup` (account ownership AO-8,
 * AO-12). Read from `GET /crew/setup` every few seconds while the window is visible, and at once
 * when Crew's change feed reports a setup job moving (WI-15). A failed read is quiet: this list
 * only adds a way to sign in. A job it showed that then failed is kept as a one-line note, so the
 * person who watched it run learns that it stopped, and why.
 */
export function useOutsideSetupJobs(): OutsideSetupJobs {
  const [jobs, setJobs] = useState<SetupRun[]>([]);
  const [failed, setFailed] = useState<SetupRun[]>([]);
  // The jobs shown as running after the last read.
  const shown = useRef<ReadonlySet<string>>(new Set());
  useEffect(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | null = null;
    let reading = false;
    let readAgain = false;
    const schedule = () => {
      if (controller.signal.aborted) return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void tick(), SETUP_JOBS_POLL_MS);
    };
    const tick = async () => {
      if (controller.signal.aborted) return;
      if (reading) {
        readAgain = true;
        return;
      }
      reading = true;
      if (visible()) {
        try {
          const listed = await listSetupJobs(controller.signal);
          if (controller.signal.aborted) return;
          const outside = listed.filter((run) => setupActive(run) && !setupStartedHere(run.jobId));
          const stopped = listed.filter(
            (run) => shown.current.has(run.jobId) && run.state === 'failed'
          );
          shown.current = new Set(outside.map((run) => run.jobId));
          if (stopped.length > 0)
            setFailed((current) => [
              ...current.filter((run) => !stopped.some((done) => done.jobId === run.jobId)),
              ...stopped,
            ]);
          setJobs((current) => (current.length === 0 && outside.length === 0 ? current : outside));
        } catch {
          // Quiet: the list only adds a way to sign in.
        }
      }
      reading = false;
      if (readAgain) {
        readAgain = false;
        void tick();
        return;
      }
      schedule();
    };
    const stopListening = onCrewChanged((delta) => {
      if (aboutSetup(delta)) void tick();
    });
    void tick();
    return () => {
      controller.abort();
      stopListening();
      if (timer) clearTimeout(timer);
    };
  }, []);
  const dismiss = useCallback(
    (jobId: string) => setFailed((current) => current.filter((run) => run.jobId !== jobId)),
    []
  );
  return { jobs, failed, dismiss };
}

/**
 * One note per setup started elsewhere that is still running (account ownership, "Desktop"): what
 * it is doing, and **Sign in** when it waits for the person. Signing in stays the person's: no
 * password or code passes through a chat, so the job waits here with its own Sign in window. A
 * setup shown here that failed leaves one line saying so, with the daemon's reason, until the
 * person dismisses it.
 */
export function ChatSetupJobNotes() {
  const { jobs, failed, dismiss } = useOutsideSetupJobs();
  const [signingIn, setSigningIn] = useState<string | null>(null);
  if (jobs.length === 0 && failed.length === 0) return null;
  return (
    <>
      {jobs.map((run) => {
        const server = jobServer(run);
        const waiting = run.state === 'needs_sign_in';
        return (
          <Note
            key={run.jobId}
            tone={waiting ? 'warning' : 'neutral'}
            role="status"
            icon={Server}
            testId="crew-outside-setup"
            action={
              waiting ? (
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  aria-label={setupJobsCopy.signInLabel(server)}
                  onClick={() => setSigningIn(run.jobId)}
                >
                  {setupJobsCopy.signIn}
                </Button>
              ) : undefined
            }
          >
            <p>{waiting ? setupJobsCopy.needsSignIn(server) : setupJobsCopy.running(server)}</p>
            {waiting ? (
              <SignInDialog
                target={{ kind: 'setup', jobId: run.jobId }}
                open={signingIn === run.jobId}
                host={server}
                onSignedIn={() => setSigningIn(null)}
                onClose={() => setSigningIn(null)}
              />
            ) : null}
          </Note>
        );
      })}
      {failed.map((run) => {
        const server = jobServer(run);
        const reason = sanitizeDisplayText(run.error?.message);
        return (
          <Note
            key={run.jobId}
            tone="danger"
            role="status"
            icon={Server}
            testId="crew-outside-setup-failed"
            action={
              <Button
                type="button"
                variant="secondary"
                size="sm"
                aria-label={setupFailedCopy.dismissLabel(server)}
                onClick={() => dismiss(run.jobId)}
              >
                {setupFailedCopy.dismiss}
              </Button>
            }
          >
            <p>
              {reason
                ? `${setupFailedCopy.failed(server)} ${reason}`
                : setupFailedCopy.failed(server)}
            </p>
          </Note>
        );
      })}
    </>
  );
}
