import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useConfig } from '../ConfigContext';
import { toastService } from '../../toasts';
import { errorMessage } from '../../utils/conversionUtils';
import { Button } from '../ui/button';
import { CopyField } from '../ui/copy-field';
import { Field } from '../ui/field';
import { Progress } from '../ui/progress';
import { Select } from '../ui/Select';
import { Spinner } from '../ui/spinner';
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
 * A warning in the panel: the shared Note's warning recipe (a wash, never a coloured border), kept
 * as a plain `role="note"` block because the public-server note is frozen privacy copy whose role
 * and test id stay as they are (class-only changes, ruling 15).
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
      setSetupError(errorMessage(error));
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
          title: ollamaCopy.detectedToast,
          msg: ollamaCopy.detectedToastDetail,
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
    setDownloadProgress({ status: ollamaCopy.startingDownload });
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
        title: ollamaCopy.connectedToast,
        msg: ollamaCopy.connectedToastDetail(selectedModel.tag),
      });
      onSuccess();
    } catch (error) {
      connectingRef.current = false;
      if (!mountedRef.current) return;
      console.error('Failed to connect to Ollama:', error);
      toastService.error({
        title: ollamaCopy.connectFailed,
        msg: ollamaCopy.connectFailedDetail(errorMessage(error)),
        traceback: error instanceof Error ? error.stack || '' : '',
      });
      setIsConnecting(false);
    }
  };

  // A host the user chose is named, so "not detected" says WHERE it looked.
  const namedHost = host !== DEFAULT_OLLAMA_HOST ? host : undefined;

  // Privacy is the provider's tier, from the daemon, never inferred here: the
  // card promised "Free, private, offline" on "your own machine" over a
  // server on another computer — and over `0.0.0.0`, which reaches this one —
  // while the tier made every chat on either Public, so a private chat could
  // not use the model the user had just been told was private. Until the
  // daemon has answered, neither claim is made.
  const isPublicServer = setup !== null && !setup.private;

  // The row this panel opens sits under Local, so a public server says what it is above
  // everything else, and the note under the status says what that means.
  const publicLabel = isPublicServer ? (
    <p className="text-supporting text-text-muted" data-testid="ollama-tier-label">
      Public · Ollama server
    </p>
  ) : null;

  const publicNote = isPublicServer ? (
    <WarningNote testId="ollama-public-note">
      Biorouter treats the Ollama server at {host} as public, not private: it counts only localhost,
      127.0.0.1 and ::1 as this computer. Chats send their prompts and files to that server, and a
      private chat cannot use it.
    </WarningNote>
  ) : null;

  const detectedPill = (
    <StatusPill tone="success">
      {ollamaCopy.running(
        namedHost,
        selectedModel && installed
          ? { tag: selectedModel.tag, installed: selectedInstalled }
          : undefined
      )}
    </StatusPill>
  );

  const notDetectedPill = (
    <StatusPill tone="warning">{ollamaCopy.notDetected(namedHost)}</StatusPill>
  );

  // Below 32 GB, or an Intel Mac: the default is still offered (there is no
  // smaller supported model), so the warning is what tells the user what to
  // expect. Unknown memory is said quietly; it is not a known problem.
  const suitabilityNote =
    recommendations && recommendations.suitability !== 'suitable' ? (
      recommendations.suitability === 'unknown_resources' ? (
        <p className="break-words text-supporting text-text-muted" data-testid="ollama-suitability">
          {recommendations.suitability_message}
        </p>
      ) : (
        <WarningNote testId="ollama-suitability">{recommendations.suitability_message}</WarningNote>
      )
    ) : null;

  const isDownloading = downloadingTag !== null;

  // A server this screen cannot ask has no state word on any option; one it can ask lists them
  // once Ollama has said what it has pulled.
  const askable = setup?.reachable ?? false;
  const modelOptions = useMemo(
    () =>
      recommendations ? modelOptionsFor(recommendations.models, askable ? installed : null) : [],
    [askable, installed, recommendations]
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
      {publicLabel}
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
            <StatusPill tone="info">
              {setup.private ? ollamaCopy.atHost(host) : ollamaCopy.onAnotherComputer(host)}
            </StatusPill>
          </div>
          {publicNote}
          <div className="space-y-2" data-testid="ollama-remote-note">
            <p className="break-words text-supporting text-text-muted">
              {ollamaCopy.remoteNote(host)}
            </p>
            <CopyField
              value={ollamaCopy.pullCommand(selectedModel?.tag ?? recommendations.default_model)}
              label={ollamaCopy.pullCommand(selectedModel?.tag ?? recommendations.default_model)}
            />
          </div>
          {setup.private && suitabilityNote}
          {modelSelect}
          {selectedModel && (
            <div className="flex flex-wrap items-center gap-2">{connectButton}</div>
          )}
        </div>
      ) : ollamaDetected ? (
        <div className="space-y-3">
          <div>{detectedPill}</div>
          {publicNote}

          {installed === null && (
            <div className="flex items-center gap-2 text-supporting text-text-muted">
              <Spinner size={14} />
              <span>{ollamaCopy.checkingModels}</span>
            </div>
          )}

          {installed !== null && modelSelect}

          {suitabilityNote}

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
          <div>{notDetectedPill}</div>
          {publicNote}
          {suitabilityNote}
          <div className="flex flex-wrap items-center gap-2">
            {isPolling ? (
              <div className="flex items-center gap-2 text-supporting text-text-muted">
                <Spinner size={14} />
                <span>{ollamaCopy.waiting}</span>
              </div>
            ) : (
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
            )}
          </div>
        </div>
      )}
    </div>
  );
}
