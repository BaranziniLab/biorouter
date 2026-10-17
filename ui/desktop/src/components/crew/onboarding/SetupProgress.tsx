import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { Check } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { Note } from '../../ui/note';
import { crewErrorCode } from '../api/errors';
import { CrewHttpError } from '../crewApi';
import { SignInDialog } from '../auth/SignInDialog';
import { sanitizeDisplayText } from '../identity';
import { hostCopy, setupCopy } from './copy';
import { useMounted } from './fields';
import { Spinner } from './parts';
import {
  manualInstallCommands,
  plainStage,
  plainStages,
  readSetup,
  readSetupArtifact,
  setupActive,
  SETUP_POLL_MS,
  setupUnsupported,
  startSetup,
  stopSetup,
  type PlainSetupStage,
  type SetupArtifact,
  type SetupInput,
  type SetupIntent,
  type SetupRun,
} from './setup';

/**
 * The renderer half of automatic setup (OW-9), shared by the Host dialog, the Not set up pane and
 * Join: a job's lifecycle (`useSetupJob`), its steps in plain words (`SetupProgress`), the Sign in
 * window a job asks for, and the manual install it falls back to (`ManualInstall`).
 */

/**
 * Where a job stands for the surface showing it. `starting`: the request is on its way.
 * `outdated`: the daemon predates setup, so the caller takes its older path. `failed` covers a
 * refused start, a job that failed or was stopped, and a read that could not reach the daemon.
 */
export type SetupJobPhase = 'idle' | 'starting' | 'running' | 'finished' | 'failed' | 'outdated';

export interface SetupProblem {
  code: string | null;
  message: string;
  /** What the server printed, when the daemon kept it. */
  output: string;
}

/** What `start` came to: the caller falls back on `outdated`, and has nothing to do otherwise. */
export type SetupStartResult = 'started' | 'outdated' | 'refused' | 'busy';

export interface SetupRetryOptions {
  replaceExisting?: boolean;
  fixStoragePermissions?: boolean;
}

/**
 * The code of a host setup that stopped before starting the workspace because other people can
 * write to a folder above Crew's storage (L3). Its message names the folders and the command; its
 * `detail` lists them. "Fix it for me" starts the job again with `fix_storage_permissions`.
 */
export const HOST_STORAGE_UNSAFE_CODE = 'crew_host_storage_unsafe';

export interface SetupJob {
  phase: SetupJobPhase;
  run: SetupRun | null;
  problem: SetupProblem | null;
  /** The job's own Sign in window is open. */
  signingIn: boolean;
  /** The job asked the person to sign in at some point (its plain steps then show Signing in). */
  askedToSignIn: boolean;
  /** Starting, or the daemon's to finish: nothing may start another. */
  busy: boolean;
  start(input: SetupInput): Promise<SetupStartResult>;
  /**
   * Start the last job again. `replaceExisting` answers `crew_install_incompatible`, and
   * `fixStoragePermissions` answers `crew_host_storage_unsafe` (the person's "Fix it for me").
   */
  retry(options?: SetupRetryOptions): Promise<SetupStartResult>;
  stop(): void;
  /** The Sign in window finished (exit 0): the job goes on by itself. */
  signedIn(): void;
  /** The Sign in window was closed before signing in: the job is stopped. */
  signInClosed(): void;
  /** Forget a job that ended (its login changed): the steps and refusal go. Not while busy. */
  reset(): void;
}

function problemOf(
  run: SetupRun,
  server: string,
  signInClosed: boolean,
  describe: ((run: SetupRun) => string | null) | undefined
): SetupProblem {
  const code = run.error?.code ?? (run.state === 'cancelled' ? 'crew_setup_cancelled' : null);
  const message =
    signInClosed && run.state === 'cancelled'
      ? setupCopy.signInEnded
      : describe?.(run) ||
        (code ? setupCopy.error(code, server) : null) ||
        sanitizeDisplayText(run.error?.message) ||
        setupCopy.failed;
  // What the server or OpenSSH said, bounded by the daemon, for the person to read or pass on.
  return { code, message, output: run.output || run.error?.detail || '' };
}

/**
 * One setup job at a time, followed to its end: started on a person's click, read again every
 * `SETUP_POLL_MS` while it runs, its Sign in window opened once when it asks for one, and
 * `onFinished` called once it finishes. Single flight: a second start while one is under way does
 * nothing (the daemon would answer the same job anyway). Unmounting stops the reading, not the job:
 * a setup stopped halfway could leave the server half set up, and starting again answers the same
 * job.
 */
export function useSetupJob({
  server,
  onFinished,
  describe,
}: {
  server: string;
  onFinished: (run: SetupRun) => void;
  /** The caller's own sentence for a job that stopped, when it has a better one than the code's. */
  describe?: (run: SetupRun) => string | null;
}): SetupJob {
  const mounted = useMounted();
  const [phase, setPhase] = useState<SetupJobPhase>('idle');
  const [run, setRun] = useState<SetupRun | null>(null);
  const [problem, setProblem] = useState<SetupProblem | null>(null);
  const [signingIn, setSigningIn] = useState(false);
  const [askedToSignIn, setAskedToSignIn] = useState(false);
  /** Each answer that comes back schedules the next read. */
  const [reads, setReads] = useState(0);
  const inFlight = useRef(false);
  const lastInput = useRef<SetupInput | null>(null);
  const askedFor = useRef(new Set<string>());
  const signInClosedFor = useRef<string | null>(null);
  const finished = useRef(onFinished);
  const serverRef = useRef(server);
  const describeRef = useRef(describe);
  useEffect(() => {
    finished.current = onFinished;
    serverRef.current = server;
    describeRef.current = describe;
  });

  const settle = useCallback((next: SetupRun) => {
    setRun(next);
    if (next.state === 'needs_sign_in' && !askedFor.current.has(next.jobId)) {
      askedFor.current.add(next.jobId);
      setAskedToSignIn(true);
      setSigningIn(true);
    }
    if (setupActive(next)) {
      setPhase('running');
      setReads((count) => count + 1);
      return;
    }
    inFlight.current = false;
    setSigningIn(false);
    if (next.state === 'finished') {
      setPhase('finished');
      finished.current(next);
      return;
    }
    setPhase('failed');
    setProblem(
      problemOf(
        next,
        serverRef.current,
        signInClosedFor.current === next.jobId,
        describeRef.current
      )
    );
  }, []);

  const start = useCallback(
    async (input: SetupInput): Promise<SetupStartResult> => {
      if (inFlight.current) return 'busy';
      inFlight.current = true;
      lastInput.current = input;
      signInClosedFor.current = null;
      setProblem(null);
      setRun(null);
      setPhase('starting');
      try {
        const status = await startSetup(input);
        if (!mounted.current) return 'started';
        settle(status);
        return 'started';
      } catch (failure) {
        inFlight.current = false;
        if (!mounted.current) return 'refused';
        if (setupUnsupported(failure)) {
          setPhase('outdated');
          return 'outdated';
        }
        const code = crewErrorCode(failure) ?? null;
        setPhase('failed');
        setProblem({
          code,
          message:
            (code ? setupCopy.error(code, serverRef.current) : null) ||
            (failure instanceof Error && failure.message ? failure.message : setupCopy.failed),
          // A refused start keeps the daemon's bounded detail too (what the server said, or the
          // folders a storage check named), for Show details.
          output: failure instanceof CrewHttpError ? (failure.detail ?? '') : '',
        });
        return 'refused';
      }
    },
    [mounted, settle]
  );

  const retry = useCallback(
    (options: SetupRetryOptions = {}) => {
      const input = lastInput.current;
      if (!input) return Promise.resolve<SetupStartResult>('refused');
      return start({
        ...input,
        ...(options.replaceExisting ? { replace_existing: true } : {}),
        ...(options.fixStoragePermissions ? { fix_storage_permissions: true } : {}),
      });
    },
    [start]
  );

  // Read a running job again until it ends.
  const jobId = phase === 'running' ? (run?.jobId ?? null) : null;
  useEffect(() => {
    if (!jobId) return;
    const controller = new AbortController();
    const timer = setTimeout(() => {
      readSetup(jobId, controller.signal).then(
        (status) => {
          if (controller.signal.aborted || !mounted.current) return;
          settle(status);
        },
        (failure: unknown) => {
          if (controller.signal.aborted || !mounted.current) return;
          inFlight.current = false;
          setSigningIn(false);
          setPhase('failed');
          setProblem({
            code: crewErrorCode(failure) ?? null,
            message:
              failure instanceof Error && failure.message ? failure.message : setupCopy.failed,
            output: '',
          });
        }
      );
    }, SETUP_POLL_MS);
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  }, [jobId, reads, mounted, settle]);

  const activeJob = run && setupActive(run) ? run.jobId : null;
  const stop = useCallback(() => {
    if (activeJob) void stopSetup(activeJob).catch(() => undefined);
  }, [activeJob]);
  const signedIn = useCallback(() => setSigningIn(false), []);
  const reset = useCallback(() => {
    if (inFlight.current) return;
    setPhase('idle');
    setRun(null);
    setProblem(null);
    setAskedToSignIn(false);
  }, []);
  const signInClosed = useCallback(() => {
    setSigningIn(false);
    if (!activeJob) return;
    signInClosedFor.current = activeJob;
    void stopSetup(activeJob).catch(() => undefined);
  }, [activeJob]);

  return {
    phase,
    run,
    problem,
    signingIn,
    askedToSignIn,
    busy: phase === 'starting' || phase === 'running',
    start,
    retry,
    stop,
    signedIn,
    signInClosed,
    reset,
  };
}

type StepState = 'done' | 'current' | 'next' | 'failed';

/** The plain steps to show: Signing in only for a job that asked for it. */
function shownStages(intent: SetupIntent, job: SetupJob): PlainSetupStage[] {
  const signIn = job.run?.stages.find((stage) => stage.name === 'sign_in');
  const signedIn =
    job.askedToSignIn ||
    (signIn !== undefined && signIn.state !== 'skipped' && signIn.state !== 'pending');
  return plainStages(intent).filter((stage) => stage !== 'signing-in' || signedIn);
}

/**
 * A job's steps in plain words — Checking server, Signing in, Installing Crew in your account,
 * Starting workspace or Connecting, Ready — with Stop while it runs, the sentence for a refusal
 * and its way on, and the job's own Sign in window. Nothing while idle or outdated: the caller
 * shows its older path then.
 */
export function SetupProgress({
  job,
  intent,
  server,
  testId,
}: {
  job: SetupJob;
  intent: SetupIntent;
  server: string;
  testId?: string;
}) {
  const fixHintId = useId();
  if (job.phase === 'idle' || job.phase === 'outdated') return null;
  const stages = shownStages(intent, job);
  const current: PlainSetupStage =
    job.phase === 'starting' || !job.run
      ? 'checking'
      : job.phase === 'finished'
        ? 'ready'
        : plainStage(job.run);
  const index = Math.max(0, stages.indexOf(current));
  const failed = job.phase === 'failed';
  const outcome = job.run?.outcome ?? null;
  const installedDetail =
    outcome?.installedVersion && outcome.reused
      ? setupCopy.reused(outcome.installedVersion)
      : outcome?.installedVersion
        ? setupCopy.installed(outcome.installedVersion)
        : null;
  const jobId = job.run?.jobId ?? null;
  return (
    <div className="crew-onboard-stack" data-testid={testId}>
      <ol className="crew-setup-steps" aria-label={setupCopy.stepsLabel}>
        {stages.map((stage, position) => {
          const state: StepState =
            job.phase === 'finished' || position < index
              ? 'done'
              : position === index
                ? failed
                  ? 'failed'
                  : 'current'
                : 'next';
          return (
            <li
              key={stage}
              className="crew-setup-step"
              data-state={state}
              aria-current={state === 'current' ? 'step' : undefined}
            >
              <span className="crew-setup-step-mark" aria-hidden="true">
                {state === 'done' ? (
                  <Check className="h-3 w-3" />
                ) : state === 'current' && stage !== 'ready' ? (
                  <Spinner />
                ) : null}
              </span>
              <span
                className={
                  state === 'next' ? 'text-body text-text-muted' : 'text-body text-text-default'
                }
              >
                {setupCopy.stages[stage]}
              </span>
              {stage === 'installing' && state === 'done' && installedDetail ? (
                <span className="text-supporting text-text-muted">{installedDetail}</span>
              ) : null}
            </li>
          );
        })}
      </ol>
      <p className="crew-onboard-row text-supporting text-text-muted" role="status">
        {job.busy ? `${setupCopy.stages[current]}…` : null}
      </p>
      {job.busy && jobId ? (
        <div className="crew-onboard-row">
          <Button type="button" variant="secondary" size="sm" onClick={job.stop}>
            {setupCopy.stop}
          </Button>
        </div>
      ) : null}
      {job.problem ? (
        <Note
          tone="warning"
          role="alert"
          testId="crew-setup-problem"
          action={
            <div className="crew-onboard-row">
              {job.problem.code === 'crew_install_incompatible' ? (
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  onClick={() => void job.retry({ replaceExisting: true })}
                >
                  {setupCopy.replace}
                </Button>
              ) : null}
              {job.problem.code === HOST_STORAGE_UNSAFE_CODE ? (
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  aria-describedby={fixHintId}
                  onClick={() => void job.retry({ fixStoragePermissions: true })}
                >
                  {setupCopy.fixStorage}
                </Button>
              ) : null}
              <Button type="button" variant="secondary" size="sm" onClick={() => void job.retry()}>
                {setupCopy.tryAgain}
              </Button>
            </div>
          }
        >
          <p>{job.problem.message}</p>
          {job.problem.code === HOST_STORAGE_UNSAFE_CODE ? (
            <p id={fixHintId} className="mt-1">
              {setupCopy.fixStorageHint}
            </p>
          ) : null}
        </Note>
      ) : null}
      {job.problem?.output ? (
        <Disclosure label={setupCopy.output}>
          <pre
            tabIndex={0}
            aria-label={setupCopy.outputLabel}
            className="crew-onboard-output font-mono text-supporting text-text-default"
          >
            {job.problem.output}
          </pre>
        </Disclosure>
      ) : null}
      {jobId ? (
        <SignInDialog
          target={{ kind: 'setup', jobId }}
          open={job.signingIn}
          host={server}
          onSignedIn={job.signedIn}
          onClose={job.signInClosed}
        />
      ) : null}
    </div>
  );
}

/**
 * The broker this build carries, as the daemon describes it, read once while `active`: what the
 * manual install shows and saves. `supported` is false for a daemon before automatic setup (its
 * routes answer 404), so callers keep to the older paths and never offer what it can't take; it
 * stays null until the daemon answered. A daemon that answers but carries no broker (a
 * development build without one staged) is supported, with no artifact.
 */
export function useSetupArtifact(active: boolean): {
  supported: boolean | null;
  artifact: SetupArtifact | null;
} {
  const [state, setState] = useState<{
    supported: boolean | null;
    artifact: SetupArtifact | null;
  }>({ supported: null, artifact: null });
  useEffect(() => {
    if (!active) return;
    const controller = new AbortController();
    readSetupArtifact(controller.signal).then(
      (artifact) => {
        if (!controller.signal.aborted) setState({ supported: true, artifact });
      },
      (failure: unknown) => {
        if (controller.signal.aborted) return;
        setState({ supported: !setupUnsupported(failure), artifact: null });
      }
    );
    return () => controller.abort();
  }, [active]);
  return state;
}

/**
 * "Install it yourself": save the broker this build carries, copy it to the server, check its
 * SHA-256 and install it, with commands generated from the artifact (OW-9). Once saved, the copy
 * line names the file where it was saved. There is no placeholder path.
 */
export function ManualInstall({
  artifact,
  route,
  server,
}: {
  artifact: SetupArtifact | null;
  route: {
    ssh_target: string;
    port?: number | null;
    proxy_jump?: string | null;
    identity_file?: string | null;
  };
  server: string;
}) {
  const mounted = useMounted();
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  if (!artifact) {
    return <p className="text-supporting text-text-muted">{setupCopy.artifactUnavailable}</p>;
  }
  const save = async () => {
    if (saving) return;
    const bridge = window.electron?.crewSaveBroker;
    if (!bridge) {
      setError(setupCopy.needsDesktop);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const result = await bridge({ version: artifact.version, sha256: artifact.sha256 });
      if (!mounted.current) return;
      if (result.success) setSaved(result.filePath);
      else if (!('canceled' in result)) setError(result.error || setupCopy.saveFailed);
    } catch {
      if (mounted.current) setError(setupCopy.saveFailed);
    } finally {
      if (mounted.current) setSaving(false);
    }
  };
  return (
    <div className="crew-onboard-stack" data-testid="crew-manual-install">
      <p className="text-body text-text-default">{setupCopy.manualIntro(server)}</p>
      <p className="text-supporting text-text-muted">
        {setupCopy.manualVersion(artifact.version, artifact.sha256)}
      </p>
      <div className="crew-onboard-row">
        <Button
          type="button"
          variant="secondary"
          size="sm"
          aria-disabled={saving || undefined}
          className="crew-onboard-waiting"
          onClick={() => void save()}
        >
          {saving ? setupCopy.saving : setupCopy.saveBroker}
        </Button>
        {saved ? (
          <span className="text-supporting text-text-muted" role="status">
            {setupCopy.saved(saved)}
          </span>
        ) : null}
      </div>
      {error ? (
        <Note tone="warning" role="alert">
          {error}
        </Note>
      ) : null}
      <CopyField
        multiline
        className="crew-onboard-command"
        label={hostCopy.installLabel}
        value={manualInstallCommands(artifact, route, saved)}
      />
    </div>
  );
}
