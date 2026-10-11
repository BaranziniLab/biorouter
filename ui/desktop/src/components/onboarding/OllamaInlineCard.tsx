import { useCallback, useEffect, useRef, useState } from 'react';
import { useConfig } from '../ConfigContext';
import { toastService } from '../../toasts';
import { Button } from '../ui/button';
import { Progress } from '../ui/progress';
import type { OllamaRecommendedModel } from '../../api';
import {
  DEFAULT_OLLAMA_HOST,
  checkOllamaStatus,
  getOllamaDownloadUrl,
  getOllamaModels,
  getOllamaSetup,
  pollForOllama,
  pullOllamaModel,
  sameOllamaTag,
  type OllamaSetup,
  type PullProgress,
} from '../../utils/ollamaDetection';
import OnboardingCardShell, { type OnboardingCardChrome } from './OnboardingCardShell';

interface OllamaInlineCardProps {
  onSuccess: () => void;
  /** See `OnboardingCardShell`. Defaults to the standalone card. */
  chrome?: OnboardingCardChrome;
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/**
 * "Qwen3.8 27B (recommended) · ~18 GB · installed". `installed` is `null` for
 * a server the card cannot ask, which then says nothing rather than guess.
 */
const optionLabel = (model: OllamaRecommendedModel, installed: boolean | null) =>
  [
    model.is_default ? `${model.display_name} (recommended)` : model.display_name,
    `~${model.approx_download_gb} GB`,
    installed === null ? null : installed ? 'installed' : 'not installed',
  ]
    .filter(Boolean)
    .join(' · ');

const Spinner = () => (
  <div className="w-3 h-3 border-2 border-current border-t-transparent rounded-full animate-spin flex-shrink-0" />
);

export default function OllamaInlineCard({ onSuccess, chrome = 'card' }: OllamaInlineCardProps) {
  const { upsert } = useConfig();
  const [isChecking, setIsChecking] = useState(true);
  const [setup, setSetup] = useState<OllamaSetup | null>(null);
  const [setupError, setSetupError] = useState<string | null>(null);
  const [ollamaDetected, setOllamaDetected] = useState(false);
  const [isPolling, setIsPolling] = useState(false);
  const [isConnecting, setIsConnecting] = useState(false);
  /** Recommended tags Ollama reports as pulled; `null` until it has been asked. */
  const [installed, setInstalled] = useState<string[] | null>(null);
  const [selectedTag, setSelectedTag] = useState<string | null>(null);
  const [downloadingTag, setDownloadingTag] = useState<string | null>(null);
  const [downloadProgress, setDownloadProgress] = useState<PullProgress | null>(null);
  const stopPollingRef = useRef<(() => void) | null>(null);
  // Every probe here is a network round trip that can settle after the card is
  // gone. Setting state then is what surfaced in vitest as `ReferenceError:
  // window is not defined` from this file's mount effect — the probe resolved
  // after the test file's jsdom had been torn down — and in the app it is a
  // write into a component nobody can see. Every state write after an await
  // checks this first.
  const mountedRef = useRef(true);
  const userChoseModelRef = useRef(false);
  const connectingRef = useRef(false);

  const refreshInstalled = useCallback(async (current: OllamaSetup) => {
    const pulled = await getOllamaModels(current.host);
    if (!mountedRef.current) return;
    const tags = current.recommendations.models
      .filter((model) => pulled.some((p) => sameOllamaTag(p.name, model.tag)))
      .map((model) => model.tag);
    setInstalled(tags);
    if (!userChoseModelRef.current) {
      // The default when it is there or nothing is; otherwise the recommended
      // model the user already has, rather than offering an 18 GB download
      // beside a model that would work now.
      const fallback = current.recommendations.default_model;
      setSelectedTag(tags.includes(fallback) || tags.length === 0 ? fallback : tags[0]);
    }
  }, []);

  const load = useCallback(async () => {
    setIsChecking(true);
    setSetupError(null);
    let current: OllamaSetup;
    try {
      current = await getOllamaSetup();
    } catch (error) {
      if (!mountedRef.current) return;
      setSetupError(errorText(error));
      setIsChecking(false);
      return;
    }
    if (!mountedRef.current) return;
    setSetup(current);
    if (!userChoseModelRef.current) setSelectedTag(current.recommendations.default_model);
    if (!current.reachable) {
      // Not probed: the CSP refuses the request, and the refusal is
      // indistinguishable from a server that is down.
      setIsChecking(false);
      return;
    }

    const status = await checkOllamaStatus(current.host);
    if (!mountedRef.current) return;
    setOllamaDetected(status.isRunning);
    if (status.isRunning) await refreshInstalled(current);
    if (!mountedRef.current) return;
    setIsChecking(false);
  }, [refreshInstalled]);

  useEffect(() => {
    mountedRef.current = true;
    void load();
    return () => {
      mountedRef.current = false;
      if (stopPollingRef.current) stopPollingRef.current();
    };
  }, [load]);

  const recommendations = setup?.recommendations ?? null;
  const host = setup?.host ?? DEFAULT_OLLAMA_HOST;
  const selectedModel =
    recommendations?.models.find((model) => model.tag === selectedTag) ??
    recommendations?.models[0] ??
    null;
  const selectedInstalled = !!selectedModel && !!installed?.includes(selectedModel.tag);

  const handleInstallClick = () => {
    if (!setup) return;
    const current = setup;
    setIsPolling(true);
    stopPollingRef.current = pollForOllama(
      async (status) => {
        if (!mountedRef.current) return;
        setOllamaDetected(status.isRunning);
        setIsPolling(false);
        await refreshInstalled(current);
        if (!mountedRef.current) return;
        toastService.success({
          title: 'Ollama Detected!',
          msg: 'Ollama is now running. You can connect to it.',
        });
      },
      3000,
      current.host
    );
  };

  const handleDownloadModel = async () => {
    if (!setup || !selectedModel) return;
    const current = setup;
    const model = selectedModel;
    setDownloadingTag(model.tag);
    setDownloadProgress({ status: 'Starting download...' });
    const result = await pullOllamaModel(
      model.tag,
      (progress) => {
        if (mountedRef.current) setDownloadProgress(progress);
      },
      current.host
    );
    // The toast is reported whether or not the card is still on screen: a
    // 19 GB pull outlives most visits to this page, and its outcome is news.
    if (result.ok) {
      toastService.success({
        title: 'Model downloaded',
        msg: `${model.display_name} (${model.tag}) is ready in Ollama.`,
      });
    } else {
      toastService.error({
        title: 'Download failed',
        msg: `Ollama could not download ${model.tag}: ${result.error}`,
        traceback: '',
      });
    }
    if (!mountedRef.current) return;
    setDownloadingTag(null);
    setDownloadProgress(null);
    await refreshInstalled(current);
  };

  const handleConnectOllama = async () => {
    if (!setup || !selectedModel || !mountedRef.current || connectingRef.current) return;
    connectingRef.current = true;
    setIsConnecting(true);
    try {
      await upsert('BIOROUTER_PROVIDER', 'ollama', false);
      if (!mountedRef.current) return;
      await upsert('BIOROUTER_MODEL', selectedModel.tag, false);
      if (!mountedRef.current) return;
      // Only when unset. This wrote `localhost` unconditionally, so finishing
      // onboarding pointed a user who had configured a remote Ollama back at
      // this machine — and the model just checked was checked on THEIR host.
      if (!setup.recommendations.host_configured) {
        await upsert('OLLAMA_HOST', 'localhost', false);
        if (!mountedRef.current) return;
      }
      toastService.success({
        title: 'Success!',
        msg: `Connected to Ollama with ${selectedModel.tag}.`,
      });
      onSuccess();
    } catch (error) {
      connectingRef.current = false;
      if (!mountedRef.current) return;
      console.error('Failed to connect to Ollama:', error);
      toastService.error({
        title: 'Connection failed',
        msg: `Failed to connect to Ollama: ${errorText(error)}`,
        traceback: error instanceof Error ? error.stack || '' : '',
      });
      setIsConnecting(false);
    }
  };

  // A host the user chose is named, so "not detected" says WHERE it looked.
  const hostSuffix = host !== DEFAULT_OLLAMA_HOST ? ` at ${host}` : '';

  // Privacy is the provider's tier, from the daemon, never inferred here: the
  // header promised "Free, private, offline" on "your own machine" over a
  // server on another computer — and over `0.0.0.0`, which reaches this one —
  // while the tier made every chat on either Public, so a private chat could
  // not use the model the user had just been told was private. Until the
  // daemon has answered, neither claim is made.
  const isPublicServer = setup !== null && !setup.private;
  const description =
    setup === null
      ? 'Run open-source models with Ollama.'
      : setup.private
        ? 'Run open-source models on your own machine. Free, private, offline.'
        : `Use open-source models on the Ollama server at ${host}. Biorouter treats it as a public model.`;

  // In the body, not only in the header: the catalog mounts this card with
  // `chrome="bare"`, which renders no header at all.
  const publicNote = isPublicServer ? (
    <div
      className="rounded-md border border-border-warning bg-background-warning/10 p-2 text-[11px] text-text-default"
      role="note"
      data-testid="ollama-public-note"
    >
      Biorouter treats the Ollama server at {host} as public, not private: it counts only localhost,
      127.0.0.1 and ::1 as this computer. Chats send their prompts and files to that server, and a
      private chat cannot use it.
    </div>
  ) : null;

  const detectedPill = (
    <span className="inline-flex items-center gap-1.5 text-[11px] text-text-muted">
      <span className="w-1.5 h-1.5 rounded-full bg-background-success" />
      Ollama running{hostSuffix}
      {selectedModel && installed
        ? ` · ${selectedModel.tag} ${selectedInstalled ? 'ready' : 'not installed'}`
        : ''}
    </span>
  );

  const notDetectedPill = (
    <span className="inline-flex items-center gap-1.5 text-[11px] text-text-muted">
      <span className="w-1.5 h-1.5 rounded-full bg-background-warning" />
      Ollama not detected{hostSuffix}
    </span>
  );

  // Below 32 GiB, or an Intel Mac: the default is still offered (there is no
  // smaller supported model), so the warning is what tells the user what to
  // expect. Unknown memory is said quietly; it is not a known problem.
  const suitabilityNote =
    recommendations && recommendations.suitability !== 'suitable' ? (
      recommendations.suitability === 'unknown_resources' ? (
        <p className="break-words text-[11px] text-text-muted" data-testid="ollama-suitability">
          {recommendations.suitability_message}
        </p>
      ) : (
        <div
          className="rounded-md border border-border-warning bg-background-warning/10 p-2 text-[11px] text-text-default"
          role="note"
          data-testid="ollama-suitability"
        >
          {recommendations.suitability_message}
        </div>
      )
    ) : null;

  const isDownloading = downloadingTag !== null;

  const modelSelect = (models: OllamaRecommendedModel[], pulled: string[] | null) => (
    <div className="min-w-0">
      <label
        htmlFor="ollama-model-select"
        className="mb-1.5 block text-xs font-medium text-text-default"
      >
        Model
      </label>
      <select
        id="ollama-model-select"
        data-testid="ollama-model-select"
        value={selectedModel?.tag ?? ''}
        onChange={(e) => {
          userChoseModelRef.current = true;
          setSelectedTag(e.target.value);
        }}
        disabled={isDownloading || isConnecting}
        className="block h-9 w-full min-w-0 max-w-full rounded-md border border-border-subtle bg-background-default px-2 text-sm text-text-default transition-colors duration-150 focus:border-border-strong"
      >
        {models.map((model) => (
          <option key={model.tag} value={model.tag}>
            {optionLabel(model, pulled === null ? null : pulled.includes(model.tag))}
          </option>
        ))}
      </select>
    </div>
  );

  const connectButton = (
    <Button
      onClick={handleConnectOllama}
      disabled={isConnecting || isDownloading}
      className="h-9 w-full px-4 sm:w-auto"
      data-testid="ollama-connect"
    >
      {isConnecting ? 'Connecting…' : 'Connect to Ollama'}
    </Button>
  );

  return (
    <OnboardingCardShell
      chrome={chrome}
      titleId="ollama-setup-title"
      category={isPublicServer ? 'commercial' : 'local'}
      label={isPublicServer ? 'Public · Ollama server' : 'Local · Run on your computer'}
      title="Ollama"
      description={description}
    >
      {isChecking ? (
        <div className="flex items-center gap-2 text-xs text-text-muted">
          <Spinner />
          <span>Checking for Ollama…</span>
        </div>
      ) : setupError || !recommendations ? (
        <div className="space-y-3">
          <p className="break-words text-xs text-text-muted" role="alert">
            {setupError ?? 'Biorouter could not read its recommended Ollama models.'}
          </p>
          <Button
            onClick={() => void load()}
            variant="outline"
            className="h-9 w-full px-4 sm:w-auto"
          >
            Try again
          </Button>
        </div>
      ) : setup && !setup.reachable ? (
        // A server the renderer cannot ask anything: another machine's
        // Ollama, or this machine's at `::1` (the CSP admits only 127.0.0.1).
        // So this branch neither lists what is pulled there nor offers a
        // download (which would land on 127.0.0.1's Ollama, if there is one).
        // The memory note is shown only for this computer: on another one the
        // figure describes the wrong machine. A chat reaches the server from
        // the daemon, so connecting is still the whole job.
        <div className="space-y-3">
          <div>
            <span className="inline-flex items-center gap-1.5 text-[11px] text-text-muted">
              <span className="w-1.5 h-1.5 rounded-full bg-background-medium" />
              {setup.private ? `Ollama at ${host}` : `Ollama on another computer · ${host}`}
            </span>
          </div>
          {publicNote}
          <p className="break-words text-xs text-text-muted" data-testid="ollama-remote-note">
            Biorouter is set to use the Ollama server at {host}. This screen can only check an
            Ollama at 127.0.0.1, so it cannot see which models that server has. Pull the model there
            first (<code>ollama pull {selectedModel?.tag ?? recommendations.default_model}</code>),
            then connect.
          </p>
          {setup.private && suitabilityNote}
          {modelSelect(recommendations.models, null)}
          {selectedModel && (
            <div className="flex flex-col items-stretch gap-2 sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-4 sm:gap-y-3">
              {connectButton}
            </div>
          )}
        </div>
      ) : ollamaDetected ? (
        <div className="space-y-3">
          <div>{detectedPill}</div>
          {publicNote}

          {installed === null && (
            <div className="flex items-center gap-2 text-xs text-text-muted">
              <Spinner />
              <span>Checking for models…</span>
            </div>
          )}

          {installed !== null && modelSelect(recommendations.models, installed)}

          {suitabilityNote}

          {isDownloading && downloadProgress && (
            <div className="rounded-md border border-border-subtle bg-background-default p-3">
              <p className="text-xs text-text-default">Downloading {downloadingTag}…</p>
              <p className="text-[11px] text-text-muted mt-1">{downloadProgress.status}</p>
              {/* Ollama's pull stream reports `total`/`completed` only once the
                  manifest resolves — before that (and during the verify/extract
                  phases) there is no denominator. That is exactly the
                  indeterminate case, and it used to render as NOTHING: the card
                  showed a status line with no bar at all, so a pull that had not
                  yet reported a size looked stalled. */}
              {downloadProgress.total && downloadProgress.completed ? (
                <>
                  <Progress
                    className="mt-2"
                    label={`Downloading ${downloadingTag}`}
                    tone="success"
                    value={(downloadProgress.completed / downloadProgress.total) * 100}
                  />
                  <p className="text-[11px] text-text-muted mt-1">
                    {Math.round((downloadProgress.completed / downloadProgress.total) * 100)}%
                  </p>
                </>
              ) : (
                <Progress
                  className="mt-2"
                  label={`Downloading ${downloadingTag}`}
                  tone="success"
                  indeterminate
                />
              )}
            </div>
          )}

          {installed !== null && selectedModel && (
            <div className="flex flex-col items-stretch gap-2 sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-4 sm:gap-y-3">
              {selectedInstalled ? (
                connectButton
              ) : (
                <Button
                  onClick={() => void handleDownloadModel()}
                  disabled={isDownloading}
                  variant="outline"
                  className="h-9 w-full px-4 sm:w-auto"
                  data-testid="ollama-download"
                >
                  Download {selectedModel.display_name} (~{selectedModel.approx_download_gb} GB)
                </Button>
              )}
            </div>
          )}
        </div>
      ) : (
        <div className="space-y-3">
          <div>{notDetectedPill}</div>
          {publicNote}
          {suitabilityNote}
          <div className="flex flex-col items-stretch gap-2 sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-4 sm:gap-y-3">
            {isPolling ? (
              <div className="flex items-center gap-2 text-xs text-text-muted">
                <Spinner />
                <span>Waiting for Ollama to start…</span>
              </div>
            ) : (
              <a
                href={getOllamaDownloadUrl()}
                target="_blank"
                rel="noopener noreferrer"
                onClick={handleInstallClick}
                className="inline-flex h-9 w-full items-center justify-center rounded-md bg-background-medium px-4 text-sm text-text-default transition-colors hover:bg-background-strong sm:w-auto"
              >
                Install Ollama
              </a>
            )}
          </div>
        </div>
      )}
    </OnboardingCardShell>
  );
}
