import { useEffect, useState } from 'react';
import { Server } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { SignInDialog } from '../auth/SignInDialog';
import { sanitizeDisplayText } from '../identity';
import { listSetupJobs, setupActive, setupStartedHere, type SetupRun } from '../onboarding/setup';
import { setupJobsCopy } from './copy';

/** How often Crew asks the daemon for setups started elsewhere, while the window is visible. */
export const SETUP_JOBS_POLL_MS = 5000;

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

/**
 * The setups this computer's daemon is running that this window did not start: a chat's
 * `crew__change setup` or `open`, or a terminal's `biorouter crew setup` (account ownership AO-8,
 * AO-12). Read from `GET /crew/setup` every few seconds while the window is visible. A failed read
 * is quiet: this list only adds a way to sign in, and the job itself says why it stopped.
 */
export function useOutsideSetupJobs(): SetupRun[] {
  const [jobs, setJobs] = useState<SetupRun[]>([]);
  useEffect(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | null = null;
    const schedule = () => {
      if (controller.signal.aborted) return;
      timer = setTimeout(() => void tick(), SETUP_JOBS_POLL_MS);
    };
    const tick = async () => {
      if (controller.signal.aborted) return;
      if (visible()) {
        try {
          const listed = await listSetupJobs(controller.signal);
          if (controller.signal.aborted) return;
          const outside = listed.filter((run) => setupActive(run) && !setupStartedHere(run.jobId));
          setJobs((current) => (current.length === 0 && outside.length === 0 ? current : outside));
        } catch {
          // Quiet: the list only adds a way to sign in.
        }
      }
      schedule();
    };
    void tick();
    return () => {
      controller.abort();
      if (timer) clearTimeout(timer);
    };
  }, []);
  return jobs;
}

/**
 * One note per setup started elsewhere that is still running (account ownership, "Desktop"): what
 * it is doing, and **Sign in** when it waits for the person. Signing in stays the person's: no
 * password or code passes through a chat, so the job waits here with its own Sign in window.
 */
export function ChatSetupJobNotes() {
  const jobs = useOutsideSetupJobs();
  const [signingIn, setSigningIn] = useState<string | null>(null);
  if (jobs.length === 0) return null;
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
    </>
  );
}
