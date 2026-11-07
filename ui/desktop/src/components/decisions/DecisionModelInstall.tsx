import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { Button } from '../ui/button';
import { Note } from '../ui/note';
import { Progress } from '../ui/progress';
import {
  DecisionRequestError,
  fetchDecisionInstall,
  installRunning,
  refusalText,
  startDecisionInstall,
  type DecisionInstallStatus,
  type InstallableDecisionModel,
} from '../../utils/sessionDecisions';
import { openInBrowser } from '../../utils/openInBrowser';
import { isBrowserSurface } from '../../utils/surface';

/** Where to get Ollama, as `utils/ollamaDetection.ts` names it. */
export const OLLAMA_DOWNLOAD_URL = 'https://ollama.com/download';

/** The model named when the daemon offers none (`OLLAMA_DEFAULT_DECISION_MODEL`). */
const DEFAULT_MODEL = 'tev1:0.8b';
const DEFAULT_SIZE = 'about 1.5 GB';

/**
 * The panel's words. `here` is "this computer", or, in a `biorouter serve`
 * browser, "the computer Biorouter runs on": the install runs in the daemon, so
 * the model lands on the daemon's computer, which is not the one in front of the
 * person there.
 */
export const DECISION_INSTALL_COPY = {
  here: 'this computer',
  hereInBrowser: 'the computer Biorouter runs on',
  why: {
    private: (here: string) =>
      `This chat is private, so only a decision model on ${here} may read it, and none is installed.`,
    crew: (here: string) =>
      `This chat works under a Crew grant, so only a decision model on ${here} may read it, and none is installed.`,
    none: 'No decision model is installed and no Baseten key is set.',
  },
  about: (model: string, size: string, here: string) =>
    `${model} is a small decision model (${size}) that runs in Ollama on ${here}.`,
  install: (model: string) => `Install ${model}`,
  installing: (model: string) => `Installing ${model}`,
  downloading: (model: string) => `Downloading ${model}…`,
  downloaded: (model: string, done: string, total: string) =>
    `Downloading ${model}… ${done} of ${total}`,
  verifying: 'Checking that Ollama answers decisions with it…',
  installed: (model: string) => `${model} is installed.`,
  retry: 'Retry',
  getOllama: 'Get Ollama',
  checkAgain: 'Check again',
  readFailed: (detail: string | null) =>
    detail
      ? `Could not read whether a decision model can be installed. ${detail}`
      : 'Could not read whether a decision model can be installed.',
};

const copy = DECISION_INSTALL_COPY;

/** Why the chat has no decision model it may use, which picks the panel's first sentence. */
export type DecisionInstallReason = 'private' | 'crew' | 'none';

/** `412 MB`, `1.5 GB`: decimal units, as Ollama and the offer's size say them. */
export function formatInstallBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 MB';
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${Math.round(bytes / 1e6)} MB`;
  return `${Math.max(1, Math.round(bytes / 1e3))} KB`;
}

/** The model an Install press installs: the daemon's default offer, else the first. */
function defaultOffer(status: DecisionInstallStatus | null): InstallableDecisionModel | null {
  if (!status) return null;
  return status.offers.find((offer) => offer.isDefault) ?? status.offers[0] ?? null;
}

export interface DecisionModelInstallProps {
  /** Why no model is usable; the first sentence. Left out, the panel opens on what it installs. */
  reason?: DecisionInstallReason | null;
  /**
   * Called once, when an install this panel watched has finished, or when the
   * daemon says the model it offers is installed already (a list read before it
   * was). The caller reads its model list again.
   */
  onInstalled?: () => void;
  /**
   * Say something only when there is something to do: nothing while reading,
   * nothing once the model is installed (unless this panel installed it), and
   * nothing where an install cannot happen (Ollama on another computer). For a
   * surface that offers the install beside other choices, such as the model
   * picker.
   */
  quiet?: boolean;
  /** How often a running install is read again. For tests; 1000 ms otherwise. */
  pollIntervalMs?: number;
}

/**
 * Installs a local decision model (`tev1:0.8b`) into the Ollama on this computer,
 * through the daemon (`POST /decisions/models/install`), so it works the same in
 * a `biorouter serve` browser, whose page cannot reach that Ollama. Offered,
 * never automatic: the download is the person's to start.
 *
 * It reads `GET /decisions/models/install` once, and every `pollIntervalMs`
 * while an install runs, showing its progress, the check that Ollama answers
 * decisions with the model, and the daemon's own sentence when it fails or when
 * Ollama is not running. A refusal because another install is running joins
 * that one.
 */
export function DecisionModelInstall({
  reason = null,
  onInstalled,
  quiet = false,
  pollIntervalMs = 1000,
}: DecisionModelInstallProps) {
  const [status, setStatus] = useState<DecisionInstallStatus | null>(null);
  const [phase, setPhase] = useState<'loading' | 'ready' | 'missing' | 'error'>('loading');
  const [readError, setReadError] = useState<string | null>(null);
  // A refused start whose sentence the status does not explain (a model it does not install).
  const [startError, setStartError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  // Whether this panel saw an install run, so its end can be told from an old one.
  const [watched, setWatched] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const notified = useRef(false);
  const onInstalledRef = useRef(onInstalled);
  onInstalledRef.current = onInstalled;

  const take = useCallback((next: DecisionInstallStatus) => {
    setStatus(next);
    if (installRunning(next.job)) {
      setWatched(true);
      setStartError(null);
    }
  }, []);

  // The first read, and Check again.
  useEffect(() => {
    const controller = new AbortController();
    fetchDecisionInstall(controller.signal)
      .then(({ data, missing }) => {
        if (controller.signal.aborted) return;
        if (missing || !data) {
          setPhase('missing');
          return;
        }
        take(data);
        setReadError(null);
        setPhase('ready');
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        setReadError(
          error instanceof DecisionRequestError
            ? error.reason
            : error instanceof Error
              ? error.message.trim() || null
              : refusalText(error)
        );
        setPhase('error');
      });
    return () => controller.abort();
  }, [attempt, take]);

  // While an install runs, read it again every `pollIntervalMs`.
  const running = installRunning(status?.job);
  useEffect(() => {
    if (!running) return;
    const controller = new AbortController();
    const timer = setTimeout(() => {
      fetchDecisionInstall(controller.signal)
        .then(({ data }) => {
          if (!controller.signal.aborted && data) take(data);
        })
        .catch(() => {
          // A missed read is retried by the next tick: keep the last progress shown.
          if (!controller.signal.aborted)
            setStatus((current) => (current ? { ...current } : current));
        });
    }, pollIntervalMs);
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  }, [status, running, pollIntervalMs, take]);

  const offer = defaultOffer(status);
  const justInstalled = watched && status?.job?.state === 'installed';
  // Installed already, but the caller's list was read before it was.
  const alreadyThere = !quiet && !watched && offer?.installed === true;
  useEffect(() => {
    if (notified.current || !(justInstalled || alreadyThere)) return;
    notified.current = true;
    onInstalledRef.current?.();
  }, [justInstalled, alreadyThere]);

  const here = isBrowserSurface() ? copy.hereInBrowser : copy.here;
  const model = offer?.model ?? DEFAULT_MODEL;

  const start = async () => {
    setStarting(true);
    setStartError(null);
    try {
      take(await startDecisionInstall(model));
      setWatched(true);
    } catch (error) {
      const sentence =
        error instanceof DecisionRequestError
          ? (error.reason ?? error.message)
          : error instanceof Error
            ? error.message
            : (refusalText(error) ?? null);
      setStartError(sentence);
      // A busy refusal joins the install that is running; Ollama stopped or
      // elsewhere is said by the status, in the daemon's own words.
      try {
        const { data } = await fetchDecisionInstall();
        if (data) take(data);
      } catch {
        // The start's own sentence stays shown.
      }
    } finally {
      setStarting(false);
    }
  };

  const job = status?.job ?? null;
  const place = status?.ollama ?? null;
  // The daemon keeps the last install's failure until the next one starts. One this
  // panel watched is said whatever followed; an older one only while the model is
  // still not installed, since it may have been installed since (`ollama pull`, or
  // `biorouter models decision pull`, which this daemon does not see).
  const failed = job?.state === 'failed' && (watched || offer?.installed !== true);

  let body: ReactNode = null;
  let actionable = true;
  if (phase === 'loading' || phase === 'missing') {
    actionable = false;
  } else if (phase === 'error') {
    actionable = false;
    body = (
      <>
        <p role="alert">{copy.readFailed(readError)}</p>
        <div>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={() => setAttempt((n) => n + 1)}
          >
            {copy.checkAgain}
          </Button>
        </div>
      </>
    );
  } else if (job && installRunning(job)) {
    const verifying = job.state === 'verifying';
    const sized = !verifying && job.totalBytes > 0;
    body = (
      <>
        <p data-testid="decision-install-progress-text">
          {verifying
            ? copy.verifying
            : sized
              ? copy.downloaded(
                  job.model,
                  formatInstallBytes(job.completedBytes),
                  formatInstallBytes(job.totalBytes)
                )
              : copy.downloading(job.model)}
        </p>
        <Progress
          size="thin"
          label={copy.installing(job.model)}
          indeterminate={!sized}
          value={sized ? job.completedBytes : 0}
          max={sized ? job.totalBytes : 100}
          minVisiblePercent={2}
          data-testid="decision-install-progress"
        />
      </>
    );
  } else if (place && place.state === 'not_running') {
    body = (
      <>
        <p>{place.message}</p>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="secondary"
            size="sm"
            data-testid="decision-install-get-ollama"
            onClick={() => void openInBrowser(OLLAMA_DOWNLOAD_URL)}
          >
            {copy.getOllama}
          </Button>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            data-testid="decision-install-check-again"
            onClick={() => {
              setStartError(null);
              setAttempt((n) => n + 1);
            }}
          >
            {copy.checkAgain}
          </Button>
        </div>
      </>
    );
  } else if (place && place.state !== 'running') {
    // Ollama on another computer, or OLLAMA_HOST unreadable: nothing to press.
    actionable = false;
    body = <p data-testid="decision-install-elsewhere">{place.message}</p>;
  } else if (startError || failed) {
    body = (
      <>
        <p role="alert" data-testid="decision-install-error">
          {startError ?? job?.error}
        </p>
        <div>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            disabled={starting}
            data-testid="decision-install-retry"
            onClick={() => void start()}
          >
            {copy.retry}
          </Button>
        </div>
      </>
    );
  } else if (justInstalled || offer?.installed) {
    actionable = justInstalled;
    body = (
      <p data-testid="decision-install-done">
        {copy.installed(justInstalled && job ? job.model : model)}
      </p>
    );
  } else {
    body = (
      <>
        <p>{copy.about(model, offer?.approxSize || DEFAULT_SIZE, here)}</p>
        <div>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            disabled={starting}
            aria-busy={starting || undefined}
            data-testid="decision-install-start"
            onClick={() => void start()}
          >
            {copy.install(model)}
          </Button>
        </div>
      </>
    );
  }

  if (quiet && !actionable) return null;
  const why =
    reason === 'private'
      ? copy.why.private(here)
      : reason === 'crew'
        ? copy.why.crew(here)
        : reason === 'none'
          ? copy.why.none
          : null;
  if (!why && !body) return null;

  return (
    <Note tone="neutral" role="status" testId="decision-model-install">
      <div className="flex flex-col gap-2">
        {why ? <p data-testid="decision-install-why">{why}</p> : null}
        {body}
      </div>
    </Note>
  );
}

export default DecisionModelInstall;
