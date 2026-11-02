import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useConfig } from '../ConfigContext';
import { toastService } from '../../toasts';
import { errorMessage } from '../../utils/conversionUtils';
import { Button } from '../ui/button';
import { CopyField } from '../ui/copy-field';
import { Field } from '../ui/field';
import { InfoTip } from '../ui/info-tip';
import { Progress } from '../ui/progress';
import { Select } from '../ui/Select';
import { Spinner } from '../ui/spinner';
import type { OllamaRecommendedModel } from '../../api';
import {
  DEFAULT_OLLAMA_HOST,
  getOllamaDownloadUrl,
  getOllamaReadiness,
  getOllamaSetup,
  pollForOllama,
  pullOllamaModel,
  sameOllamaTag,
  type OllamaSetup,
  type ConfiguredOllamaReadiness,
  type PullProgress,
} from '../../utils/ollamaDetection';
import { localModelsCopy } from '../settings/models/copy';
import { ollamaFit } from '../settings/models/localModelMemory';
import './onboarding.css';
import { ollamaCopy } from './copy';
import { StatusPill } from './StatusPill';
import { useViewPrimaryVariant } from './viewPrimary';

interface OllamaInlineCardProps {
  onSuccess: () => void;
}

type ModelOption = { value: string; label: string };

/**
 * "Qwen3.8 27B (recommended) · ~18 GB · Installed". `installed` is `null` for a server the card
 * cannot ask, which then says nothing rather than guess.
 */
const optionLabel = (model: OllamaRecommendedModel, installed: boolean | null) =>
  [
    model.is_default ? ollamaCopy.recommended(model.display_name) : model.display_name,
    ollamaCopy.approxSize(model.approx_download_gb),
    installed === null ? null : installed ? ollamaCopy.installed : ollamaCopy.needsDownload,
  ]
    .filter(Boolean)
    .join(' · ');

const modelOptionsFor = (
  models: OllamaRecommendedModel[],
  pulled: string[] | null
): ModelOption[] =>
  models.map((model) => ({
    value: model.tag,
    label: optionLabel(model, pulled === null ? null : pulled.includes(model.tag)),
  }));

/**
 * The configured server as the user wrote it (`base_url`, `OLLAMA_HOST` resolved by the daemon),
 * without its trailing slash: the address every word on the panel names. `OllamaSetup.host` is
 * where the renderer PROBES that server, which turns `0.0.0.0` and `localhost` into 127.0.0.1 for
 * the CSP. Naming the probe address made the public note say Biorouter "treats the Ollama server
 * at http://127.0.0.1:11434 as public ... it counts only localhost, 127.0.0.1 and ::1 as this
 * computer" for `OLLAMA_HOST=0.0.0.0`, which contradicts itself.
 */
function configuredHost(baseUrl: string): string {
  try {
    const url = new URL(baseUrl);
    return `${url.origin}${url.pathname}`.replace(/\/+$/, '');
  } catch {
    return baseUrl.replace(/\/+$/, '');
  }
}

/**
 * A warning in the panel: the shared Note's warning recipe (a wash, never a coloured border), kept
 * as a plain `role="note"` block because the public-server note is frozen privacy copy whose role
 * and test id stay as they are (class-only changes, ruling 15). Its words are unchanged; the host
 * it names is the configured one (`configuredHost`).
 */
function WarningNote({ testId, children }: { testId: string; children: React.ReactNode }) {
  return (
    <div
      className="br-fill-surface rounded-element bg-wash-warning px-3 py-2.5 text-supporting text-text-warning [overflow-wrap:anywhere]"
      role="note"
      data-testid={testId}
    >
      {children}
    </div>
  );
}

export default function OllamaInlineCard({ onSuccess }: OllamaInlineCardProps) {
  const { upsert } = useConfig();
  // Coral unless another open panel in the tab holds the view's primary (FR-R5-2).
  const primaryVariant = useViewPrimaryVariant();
  const [isChecking, setIsChecking] = useState(true);
  const [setup, setSetup] = useState<OllamaSetup | null>(null);
  const [setupError, setSetupError] = useState<string | null>(null);
  const [readiness, setReadiness] = useState<ConfiguredOllamaReadiness | null>(null);
  const [readinessError, setReadinessError] = useState<string | null>(null);
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
  const probeGenerationRef = useRef(0);
  const userChoseModelRef = useRef(false);
  const connectingRef = useRef(false);

  const refreshInstalled = useCallback(async (current: OllamaSetup, generation: number) => {
    let checked: ConfiguredOllamaReadiness;
    try {
      checked = await getOllamaReadiness();
    } catch (error) {
      if (!mountedRef.current || generation !== probeGenerationRef.current) return;
      setOllamaDetected(false);
      setReadiness(null);
      setInstalled(null);
      setReadinessError(errorMessage(error));
      return;
    }
    if (!mountedRef.current || generation !== probeGenerationRef.current) return;
    setReadiness(checked);
    setReadinessError(checked.error ?? null);
    setOllamaDetected(checked.reachable);
    const tags = current.recommendations.models
      .filter((model) => checked.models.some((p) => sameOllamaTag(p.name, model.tag)))
      .map((model) => model.tag);
    setInstalled(tags);
    if (!userChoseModelRef.current) {
      // The default when it is there or nothing is; otherwise the recommended
      // model the user already has, rather than offering an 18 GB download
      // beside a model that would work now.
      const fallback = current.recommendations.default_model;
      setSelectedTag(tags.includes(fallback) || tags.length === 0 ? fallback : tags[0]);
    }
    return checked.reachable;
  }, []);

  const load = useCallback(async () => {
    const generation = ++probeGenerationRef.current;
    setIsChecking(true);
    setSetupError(null);
    setReadinessError(null);
    stopPollingRef.current?.();
    setIsPolling(false);
    connectingRef.current = false;
    setIsConnecting(false);
    let current: OllamaSetup;
    try {
      current = await getOllamaSetup();
    } catch (error) {
      if (!mountedRef.current || generation !== probeGenerationRef.current) return;
      setSetupError(errorMessage(error));
      setIsChecking(false);
      return;
    }
    if (!mountedRef.current || generation !== probeGenerationRef.current) return;
    setSetup(current);
    if (!userChoseModelRef.current) setSelectedTag(current.recommendations.default_model);
    await refreshInstalled(current, generation);
    if (!mountedRef.current || generation !== probeGenerationRef.current) return;
    setIsChecking(false);
  }, [refreshInstalled]);

  useEffect(() => {
    mountedRef.current = true;
    void load();
    return () => {
      mountedRef.current = false;
      probeGenerationRef.current += 1;
      if (stopPollingRef.current) stopPollingRef.current();
    };
  }, [load]);

  const recommendations = setup?.recommendations ?? null;
  // Where the renderer probes the server, for fetches only (see `configuredHost`).
  const host = setup?.host ?? DEFAULT_OLLAMA_HOST;
  // The server the user configured: the one every status line and note names.
  const shownHost = recommendations ? configuredHost(recommendations.base_url) : host;
  const selectedModel =
    recommendations?.models.find((model) => model.tag === selectedTag) ??
    recommendations?.models[0] ??
    null;
  const selectedInstalled = !!selectedModel && !!installed?.includes(selectedModel.tag);

  const handleInstallClick = () => {
    if (!setup?.reachable) return;
    const current = setup;
    const generation = ++probeGenerationRef.current;
    stopPollingRef.current?.();
    setIsPolling(true);
    stopPollingRef.current = pollForOllama(
      async (status) => {
        if (!mountedRef.current || generation !== probeGenerationRef.current) return;
        setOllamaDetected(status.isRunning);
        setIsPolling(false);
        const available = await refreshInstalled(current, generation);
        if (!mountedRef.current || generation !== probeGenerationRef.current || !available) return;
        toastService.success({
          title: ollamaCopy.detectedToast,
          msg: ollamaCopy.detectedToastDetail,
        });
      },
      3000,
      current.host,
      async () => {
        try {
          const checked = await getOllamaReadiness();
          return { isRunning: checked.reachable, host: checked.base_url };
        } catch {
          return { isRunning: false, host: current.host };
        }
      }
    );
  };

  const handleDownloadModel = async () => {
    if (!setup?.reachable || !selectedModel) return;
    const current = setup;
    const generation = probeGenerationRef.current;
    const model = selectedModel;
    setDownloadingTag(model.tag);
    setDownloadProgress({ status: ollamaCopy.startingDownload });
    const result = await pullOllamaModel(
      model.tag,
      (progress) => {
        if (mountedRef.current && generation === probeGenerationRef.current)
          setDownloadProgress(progress);
      },
      current.host
    );
    // The toast is reported whether or not the card is still on screen: a
    // 19 GB pull outlives most visits to this page, and its outcome is news.
    if (result.ok) {
      toastService.success({
        title: ollamaCopy.downloadedToast,
        msg: ollamaCopy.downloadedToastDetail(model.display_name, model.tag),
      });
    } else {
      toastService.error({
        title: ollamaCopy.downloadFailed,
        msg: ollamaCopy.downloadFailedDetail(model.tag, result.error),
        traceback: '',
      });
    }
    if (!mountedRef.current) return;
    setDownloadingTag(null);
    setDownloadProgress(null);
    if (generation !== probeGenerationRef.current) return;
    await refreshInstalled(current, generation);
  };

  const handleConnectOllama = async () => {
    if (!setup || !selectedModel || !mountedRef.current || connectingRef.current) return;
    connectingRef.current = true;
    const generation = ++probeGenerationRef.current;
    setIsConnecting(true);
    setReadinessError(null);
    try {
      const checked = await getOllamaReadiness(selectedModel.tag);
      if (!mountedRef.current || generation !== probeGenerationRef.current) return;
      setReadiness(checked);
      setOllamaDetected(checked.reachable);
      setInstalled(
        setup.recommendations.models
          .filter((model) => checked.models.some((pulled) => sameOllamaTag(pulled.name, model.tag)))
          .map((model) => model.tag)
      );
      if (
        !checked.reachable ||
        checked.model_available !== true ||
        checked.supports_tools === false
      ) {
        throw new Error(checked.error || `Ollama is not ready to use ${selectedModel.tag}.`);
      }
      await upsert('BIOROUTER_PROVIDER', 'ollama', false);
      if (!mountedRef.current || generation !== probeGenerationRef.current) return;
      await upsert('BIOROUTER_MODEL', selectedModel.tag, false);
      if (!mountedRef.current || generation !== probeGenerationRef.current) return;
      // Only when unset. This wrote `localhost` unconditionally, so finishing
      // onboarding pointed a user who had configured a remote Ollama back at
      // this machine — and the model just checked was checked on THEIR host.
      if (!setup.recommendations.host_configured) {
        await upsert('OLLAMA_HOST', 'localhost', false);
        if (!mountedRef.current || generation !== probeGenerationRef.current) return;
      }
      toastService.success({
        title: ollamaCopy.connectedToast,
        msg: ollamaCopy.connectedToastDetail(selectedModel.display_name),
      });
      onSuccess();
    } catch (error) {
      if (!mountedRef.current || generation !== probeGenerationRef.current) return;
      connectingRef.current = false;
      setReadinessError(errorMessage(error));
      console.error('Failed to connect to Ollama:', error);
      toastService.error({
        title: ollamaCopy.connectFailed,
        msg: ollamaCopy.connectFailedDetail(errorMessage(error)),
        traceback: error instanceof Error ? error.stack || '' : '',
      });
      setIsConnecting(false);
    }
  };

  // Privacy is the provider's tier, from the daemon, never inferred here: the
  // card promised "Free, private, offline" on "your own machine" over a
  // server on another computer — and over `0.0.0.0`, which reaches this one —
  // while the tier made every chat on either Public, so a private chat could
  // not use the model the user had just been told was private. Until the
  // daemon has answered, neither claim is made.
  const isPublicServer = setup !== null && !setup.private;

  // A host the user chose is named, so "not detected" says WHERE it looked. Only the default
  // server (this computer's, at 11434) goes unnamed. `0.0.0.0` probes at that same address, so a
  // public server is always named: its status line said "Public · Ollama running" with no host.
  const namedHost = host !== DEFAULT_OLLAMA_HOST || isPublicServer ? shownHost : undefined;

  // The row this panel opens sits under Local, whose tier line says nothing leaves this machine,
  // so a public server's status line says "Public" first, and the note under it says what that
  // means. One status line, not a caption over it.
  const status = (tone: 'success' | 'warning' | 'info', words: string) => (
    <div>
      <StatusPill tone={tone}>
        <span>
          {isPublicServer ? (
            <>
              <span data-testid="ollama-tier-label">{ollamaCopy.publicTier}</span>
              {' · '}
            </>
          ) : null}
          <span>{words}</span>
        </span>
      </StatusPill>
    </div>
  );

  const publicNote = isPublicServer ? (
    <WarningNote testId="ollama-public-note">{ollamaCopy.publicNote(shownHost)}</WarningNote>
  ) : null;

  // Below 32 GB, or an Intel Mac: the default is still offered (there is no
  // smaller supported model), so the warning is what tells the user what to
  // expect. The word is the Llama Server rows' ("Below the 32 GB minimum", "CPU only, slow"), in
  // warning ink; why is its InfoTip, in GB. Unknown memory is said quietly: it is not a known
  // problem. The daemon's GiB paragraph used to be a warning box here (FR-R4-5, principle 2).
  const fit = recommendations ? ollamaFit(recommendations) : null;
  const suitabilityNote = fit ? (
    <p
      className="flex min-w-0 flex-wrap items-center text-supporting text-text-muted"
      role="note"
      data-testid="ollama-suitability"
    >
      <span className={fit.warning ? 'text-text-warning' : undefined}>{fit.word}</span>
      <InfoTip label={localModelsCopy.memoryLabel} help={fit.help} />
    </p>
  ) : null;

  const isDownloading = downloadingTag !== null;

  const modelOptions = useMemo(
    () => (recommendations ? modelOptionsFor(recommendations.models, installed) : []),
    [installed, recommendations]
  );

  // The app's one select (`ui/Select`), as Llama Server's panel draws it: react-select names its
  // own input, so the label points at `inputId`, and the test id sits on a wrapper because the
  // library forwards no `data-*` attribute. The menu is portalled so a scrolling page never clips it.
  const modelSelect = (
    <Field id="ollama-model-select" label={ollamaCopy.model}>
      <div id="ollama-model-field" data-testid="ollama-model-select">
        <Select
          inputId="ollama-model-select"
          options={modelOptions}
          value={modelOptions.find((option) => option.value === selectedModel?.tag) ?? null}
          onChange={(option: unknown) => {
            const chosen = option as ModelOption | null;
            if (!chosen) return;
            userChoseModelRef.current = true;
            setSelectedTag(chosen.value);
            setReadiness(null);
            setReadinessError(null);
          }}
          isDisabled={isDownloading || isConnecting}
          isSearchable={false}
          menuPlacement="auto"
          menuPortalTarget={typeof document === 'undefined' ? undefined : document.body}
        />
      </div>
    </Field>
  );

  const connectButton = (
    <Button
      variant={primaryVariant}
      onClick={() => void handleConnectOllama()}
      disabled={isConnecting || isDownloading}
      data-testid="ollama-connect"
    >
      {isConnecting ? ollamaCopy.connecting : ollamaCopy.use}
    </Button>
  );

  return (
    <div className="min-w-0 space-y-3">
      {isChecking ? (
        <div className="flex items-center gap-2 text-supporting text-text-muted">
          <Spinner size={14} />
          <span>{ollamaCopy.checking}</span>
        </div>
      ) : setupError || !recommendations ? (
        <div className="space-y-3">
          <p className="break-words text-supporting text-text-muted" role="alert">
            {setupError ?? ollamaCopy.noRecommendations}
          </p>
          <div className="flex flex-wrap items-center gap-2">
            <Button variant="secondary" onClick={() => void load()}>
              {ollamaCopy.tryAgain}
            </Button>
          </div>
        </div>
      ) : setup && !setup.reachable && ollamaDetected ? (
        // The daemon can check this server; downloads still require a direct
        // renderer connection, so offer its pull command instead.
        <div className="space-y-3">
          {status('success', ollamaCopy.running(shownHost))}
          {publicNote}
          {modelSelect}
          {setup.private && suitabilityNote}
          {!selectedInstalled && (
            <div className="space-y-2" data-testid="ollama-remote-note">
              <p className="flex min-w-0 flex-wrap items-center text-supporting text-text-muted">
                <span className="min-w-0">{ollamaCopy.remoteInstruction}</span>
                <InfoTip label={ollamaCopy.remoteTipLabel} help={ollamaCopy.remoteTip} />
              </p>
              <CopyField
                value={ollamaCopy.pullCommand(selectedModel?.tag ?? recommendations.default_model)}
                label={ollamaCopy.pullCommand(selectedModel?.tag ?? recommendations.default_model)}
              />
              <Button variant="secondary" onClick={() => void load()}>
                Check again
              </Button>
            </div>
          )}
          {selectedModel && selectedInstalled && (
            <div className="flex flex-wrap items-center gap-2">{connectButton}</div>
          )}
        </div>
      ) : ollamaDetected ? (
        <div className="space-y-3">
          {status('success', ollamaCopy.running(namedHost))}
          {publicNote}

          {installed === null && (
            <div className="flex items-center gap-2 text-supporting text-text-muted">
              <Spinner size={14} />
              <span>{ollamaCopy.checkingModels}</span>
            </div>
          )}

          {installed !== null && modelSelect}

          {setup?.private && suitabilityNote}

          {downloadingTag !== null && downloadProgress && (
            <div className="br-first-run-box br-fill-surface p-3">
              <p className="text-supporting text-text-default">
                {ollamaCopy.downloading(downloadingTag)}
              </p>
              <p className="text-supporting text-text-muted mt-1">{downloadProgress.status}</p>
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
                    label={ollamaCopy.downloading(downloadingTag)}
                    tone="success"
                    value={(downloadProgress.completed / downloadProgress.total) * 100}
                  />
                  <p className="text-supporting text-text-muted mt-1">
                    {Math.round((downloadProgress.completed / downloadProgress.total) * 100)}%
                  </p>
                </>
              ) : (
                <Progress
                  className="mt-2"
                  label={ollamaCopy.downloading(downloadingTag)}
                  tone="success"
                  indeterminate
                />
              )}
            </div>
          )}

          {/* One action on the 32px rung, never stretched; the model's name is in the picker
              above, so the button says what it does and how much it downloads (FR-S-8). */}
          {installed !== null && selectedModel && (
            <div className="flex flex-wrap items-center gap-2">
              {selectedInstalled ? (
                connectButton
              ) : (
                <Button
                  variant="secondary"
                  onClick={() => void handleDownloadModel()}
                  disabled={isDownloading}
                  data-testid="ollama-download"
                >
                  {ollamaCopy.downloadModel(selectedModel.approx_download_gb)}
                </Button>
              )}
            </div>
          )}
        </div>
      ) : (
        <div className="space-y-3">
          {status('warning', ollamaCopy.notDetected(namedHost))}
          {publicNote}
          {setup?.private && suitabilityNote}
          <div className="flex flex-wrap items-center gap-2">
            {isPolling ? (
              <div className="flex items-center gap-2 text-supporting text-text-muted">
                <Spinner size={14} />
                <span>{ollamaCopy.waiting}</span>
              </div>
            ) : setup?.reachable ? (
              <Button asChild variant="secondary">
                <a
                  href={getOllamaDownloadUrl()}
                  target="_blank"
                  rel="noopener noreferrer"
                  onClick={handleInstallClick}
                >
                  {ollamaCopy.install}
                </a>
              </Button>
            ) : null}
            <Button variant="secondary" onClick={() => void load()}>
              Check again
            </Button>
          </div>
        </div>
      )}
      {readinessError && !isChecking && (
        <div className="space-y-2">
          <p className="break-words text-supporting text-text-muted" role="alert">
            {readinessError}
          </p>
          <p className="text-supporting text-text-muted">
            The host address tells Biorouter where to connect; it does not start Ollama.
            {readiness?.reachable
              ? ' Check the selected model on that server, then choose Check again.'
              : ' Open Ollama on the computer at that address, then choose Check again.'}
          </p>
          {ollamaDetected && (
            <Button variant="secondary" onClick={() => void load()}>
              Check again
            </Button>
          )}
        </div>
      )}
      {readiness?.selected_model === selectedModel?.tag && readiness?.context_window != null && (
        <p className="text-supporting text-text-muted">
          Biorouter will request a context window of {readiness.context_window.toLocaleString()}{' '}
          tokens. A larger window uses more memory and takes longer to process.
        </p>
      )}
    </div>
  );
}
