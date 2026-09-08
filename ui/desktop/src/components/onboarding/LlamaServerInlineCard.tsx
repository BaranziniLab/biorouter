import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useConfig } from '../ConfigContext';
import { toastService } from '../../toasts';
import { Button } from '../ui/button';
import { llamacppEnsure, llamacppWarmup, type LlamaCppModel } from '../../api';
import { llamaServerStore, useLlamaServer } from '../settings/models/llamaServerStore';
import OnboardingCardShell, { type OnboardingCardChrome } from './OnboardingCardShell';
import { ConfirmationModal } from '../ui/ConfirmationModal';

interface LlamaServerInlineCardProps {
  onSuccess: () => void;
  /** See `OnboardingCardShell`. Defaults to the standalone card. */
  chrome?: OnboardingCardChrome;
}

const acceleratorMemoryLabel = (kind: string | undefined) =>
  kind === 'apple_unified' ? 'unified memory' : 'VRAM';

const acceleratorMemoryExplanation = (kind: string | undefined) =>
  kind === 'apple_unified'
    ? 'On Apple Silicon, unified memory is the relevant GPU memory budget.'
    : 'On Intel Macs, Windows, and other discrete-GPU systems, this means VRAM, not regular system RAM.';

const modelDownloadLabel = (model: LlamaCppModel | undefined) => {
  switch (model?.download_status) {
    case 'downloaded':
      return model.download_source === 'ollama' ? 'Downloaded in Ollama' : 'Downloaded';
    case 'partial':
      return 'Partial download';
    default:
      return 'Needs download';
  }
};

const modelDownloadNote = (model: LlamaCppModel | undefined) => {
  switch (model?.download_status) {
    case 'downloaded':
      return model.download_source === 'ollama'
        ? 'Already pulled by Ollama; Llama Server tries that local blob first and falls back to a compatible GGUF if needed.'
        : 'Already downloaded on this machine; startup should only need model load and warm-up.';
    case 'partial':
      return 'A previous download is incomplete; startup will resume or redownload before loading.';
    default:
      return 'Not downloaded yet; first startup may take a while before warm-up can run.';
  }
};

const terminalErrorTitle = (kind: 'install' | 'start' | 'warmup') =>
  kind === 'warmup'
    ? 'Llama Server warm-up failed'
    : kind === 'install'
      ? 'Local model install failed'
      : 'Could not start Llama Server';

const fallbackDownloadLabel = (model: LlamaCppModel | undefined) => {
  switch (model?.fallback_download_status) {
    case 'downloaded':
      return 'Fallback ready';
    case 'partial':
      return 'Fallback partial';
    case 'not_downloaded':
      return model?.ollama_name ? 'Fallback may download' : null;
    default:
      return null;
  }
};

export default function LlamaServerInlineCard({
  onSuccess,
  chrome = 'card',
}: LlamaServerInlineCardProps) {
  const { upsert } = useConfig();
  const { status, operation, lastError } = useLlamaServer();
  // Skip the "Checking…" state when the shared store already has a status
  // (e.g. remounting while a download started elsewhere is still running).
  const [isChecking, setIsChecking] = useState(
    () => llamaServerStore.getSnapshot().status === null
  );
  const [selectedModel, setSelectedModel] = useState<string>('');
  const [isConnecting, setIsConnecting] = useState(false);
  const [pendingModelStart, setPendingModelStart] = useState<string | null>(null);
  const connectingRef = useRef(false);
  // Ownership guard: the start flow deliberately keeps driving the shared
  // store after this card unmounts (progress lives in the store, issue #34),
  // but a DEAD card must never write provider config, call its own state
  // setters, or fire the onSuccess navigation.
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const sidecar = status?.sidecar ?? null;
  const catalog = useMemo(() => status?.catalog ?? [], [status]);
  const system = status?.system ?? null;
  // Any in-flight operation (started here or in Settings → Models) renders
  // the live progress box; the store keeps polling across unmounts.
  const isStarting = operation !== null;

  const connect = useCallback(
    async (model: string) => {
      // Never configure the provider or navigate from an unmounted card.
      if (!mountedRef.current) return;
      if (connectingRef.current) return;
      connectingRef.current = true;
      setIsConnecting(true);
      try {
        // Each write is awaited, so the card can unmount between any two of
        // them; re-check ownership after EVERY await so a dead card stops
        // writing provider config mid-sequence instead of finishing the
        // group from beyond the grave.
        // Explicitly setting the (defaulted) port marks the provider configured.
        //
        // The NUMBER 11543, never the string '11543'. `/config/upsert` writes the
        // value verbatim, and the backend reads this key back with a typed
        // `Config::get_param::<usize>()` — serde_yaml does not coerce a quoted
        // scalar into a `usize`, so `LLAMACPP_PORT: '11543'` deserialised as `Err`
        // and every call site swallowed it with `.ok()`/`.unwrap_or(DEFAULT)`: the
        // key saved, and did nothing. Pinned by LlamaServerInlineCard.test.tsx.
        await upsert('LLAMACPP_PORT', 11543, false);
        if (!mountedRef.current) return;
        await upsert('BIOROUTER_PROVIDER', 'llamacpp', false);
        if (!mountedRef.current) return;
        await upsert('BIOROUTER_MODEL', model, false);
        // Unmounted after the last write: the config is saved, but a dead
        // card must not toast or drive the onboarding navigation.
        if (!mountedRef.current) return;
        toastService.success({
          title: 'Local model ready!',
          msg: `Llama Server is running ${model} on your computer.`,
        });
        onSuccess();
      } catch (error) {
        connectingRef.current = false;
        if (mountedRef.current) setIsConnecting(false);
        toastService.error({
          title: 'Connection failed',
          msg: `Failed to configure Llama Server: ${error instanceof Error ? error.message : String(error)}`,
          traceback: error instanceof Error ? error.stack || '' : '',
        });
      }
    },
    [onSuccess, upsert]
  );

  useEffect(() => {
    let cancelled = false;
    const checkInitial = async () => {
      try {
        await llamaServerStore.refresh();
      } catch (error) {
        console.error('Failed to check Llama Server status:', error);
      } finally {
        if (!cancelled) setIsChecking(false);
      }
    };
    void checkInitial();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!status) return;
    const defaultModel =
      status.catalog.find((m) => m.is_default)?.name ?? status.catalog[0]?.name ?? '';
    setSelectedModel((prev) => prev || status.sidecar.model || defaultModel);
  }, [status]);

  // Surface a polled terminal failure (sidecar error / deadline timeout)
  // IMMEDIATELY, instead of leaving the user staring at a silently cleared
  // busy state until the separate in-flight warm-up HTTP call gets around to
  // rejecting. claimErrorToast makes this exactly-once across surfaces and
  // the driving flow's own catch handler.
  useEffect(() => {
    if (!lastError) return;
    if (!llamaServerStore.claimErrorToast(lastError.opId)) return;
    toastService.error({
      title: terminalErrorTitle(lastError.kind),
      msg: lastError.message,
    });
  }, [lastError]);

  // Toast a driving-flow failure exactly once. If the store already failed
  // this operation terminally (polled sidecar error / timeout), the effect
  // above may have surfaced it — claim before toasting the retained error.
  // A flow superseded by a newer operation stays silent: the new flow owns
  // the UX, and its operation/interval must not be touched (scoped
  // endOperation guarantees that).
  const surfaceStartFailure = (opId: number, title: string, error: unknown) => {
    const endedByUs = llamaServerStore.endOperation(opId);
    if (!endedByUs) {
      const terminal = llamaServerStore.getSnapshot().lastError;
      if (terminal?.opId === opId && llamaServerStore.claimErrorToast(opId)) {
        toastService.error({ title: terminalErrorTitle(terminal.kind), msg: terminal.message });
      }
      return;
    }
    toastService.error({
      title,
      msg: error instanceof Error ? error.message : String(error),
      traceback: error instanceof Error ? error.stack || '' : '',
    });
  };

  const startModel = async (model: string) => {
    if (llamaServerStore.getSnapshot().operation || isConnecting) return;
    const opId = llamaServerStore.beginOperation('start', model);
    try {
      const res = await llamacppEnsure({ body: { model }, throwOnError: true });
      llamaServerStore.applyStatus(res.data, opId);
    } catch (error) {
      surfaceStartFailure(opId, 'Could not start Llama Server', error);
      return;
    }

    // The ensure call may settle after this flow lost the operation (deadline
    // timeout, or a newer operation superseded it — possibly for a DIFFERENT
    // model). applyStatus above already dropped the stale payload, but the
    // flow itself must also stop: warming up the OLD model here would
    // interfere with the singleton sidecar the newer operation now owns.
    if (llamaServerStore.getSnapshot().operation?.id !== opId) return;

    try {
      const warmed = await llamacppWarmup({ body: { model }, throwOnError: true });
      if (!warmed.data.output.trim()) {
        throw new Error('Llama Server returned an empty warm-up response');
      }
      llamaServerStore.applySidecar(warmed.data.sidecar, opId);
      // Scoped endOperation: false means this flow was superseded or already
      // failed terminally in the store — a stale flow must not connect.
      if (!llamaServerStore.endOperation(opId)) return;
      // A dead card never writes provider config or navigates; the model is
      // ready in the store, and a remounted card will offer "Use Llama
      // Server" from the ready status.
      if (!mountedRef.current) return;
      await connect(model);
    } catch (error) {
      surfaceStartFailure(opId, 'Llama Server warm-up failed', error);
    }
  };

  const selectedEntry = catalog.find((m) => m.name === selectedModel);
  const modelMemoryWarning = useMemo(() => {
    if (!selectedEntry || !system) return null;
    const detected = system.accelerator_memory_gib;
    if (typeof detected === 'number' && detected >= selectedEntry.recommended_gpu_memory_gib) {
      return null;
    }
    const detectedText =
      typeof detected === 'number'
        ? `${detected} GiB ${acceleratorMemoryLabel(system.accelerator_memory_kind)}`
        : `unknown ${acceleratorMemoryLabel(system.accelerator_memory_kind)}`;
    return `${selectedEntry.display_name} recommends ${selectedEntry.recommended_gpu_memory_gib} GiB GPU-addressable memory. This machine reports ${detectedText}. ${acceleratorMemoryExplanation(system.accelerator_memory_kind)}`;
  }, [selectedEntry, system]);

  const handleStart = (model: string) => {
    if (isStarting || isConnecting) return;
    if (modelMemoryWarning) {
      setPendingModelStart(model);
      return;
    }
    void startModel(model);
  };

  const resourceWarnings = useMemo(() => {
    if (!system || !selectedEntry) return [];

    const warnings: string[] = [];
    const detected = system.accelerator_memory_gib;
    const memoryLabel = acceleratorMemoryLabel(system.accelerator_memory_kind);
    if (typeof detected === 'number' && detected < selectedEntry.recommended_gpu_memory_gib) {
      warnings.push(
        `This machine reports ${detected} GiB ${memoryLabel}; ${selectedEntry.display_name} recommends ${selectedEntry.recommended_gpu_memory_gib} GiB GPU-addressable memory.`
      );
    } else if (detected == null) {
      warnings.push(
        `Biorouter could not detect VRAM; ${selectedEntry.display_name} recommends ${selectedEntry.recommended_gpu_memory_gib} GiB GPU-addressable memory.`
      );
    }
    if (selectedEntry.recommended_gpu_memory_gib > 16) {
      warnings.push(
        `This model is above the 16 GB laptop tier; Gemma 4 is the laptop default. ${acceleratorMemoryExplanation(system.accelerator_memory_kind)}`
      );
    }
    if (system.os.toLowerCase().includes('windows')) {
      warnings.push(
        'Windows needs enough free VRAM for the selected model and context window; regular system RAM does not satisfy the GPU memory recommendation.'
      );
    }
    return warnings;
  }, [selectedEntry, system]);
  const isRunningForSelected = sidecar?.state === 'ready' && sidecar.model === selectedModel;
  const isReadyForSelected = isRunningForSelected && sidecar?.warmed;
  const binaryMissing = sidecar?.state === 'no_binary';
  const startButtonLabel = (() => {
    if (isStarting) return 'Setting up…';
    if (isRunningForSelected) return 'Warm up model';
    if (
      selectedEntry?.download_status === 'downloaded' &&
      selectedEntry.fallback_download_status === 'not_downloaded'
    ) {
      return 'Run & warm up (fallback may download)';
    }
    if (selectedEntry?.download_status === 'downloaded') return 'Run & warm up';
    if (selectedEntry?.download_status === 'partial') {
      return `Resume download & run (${selectedEntry.download_size})`;
    }
    return `Download & run${selectedEntry ? ` (${selectedEntry.download_size})` : ''}`;
  })();

  const statusPill = (() => {
    if (isChecking || !sidecar) return null;
    if (binaryMissing) {
      return (
        <span className="inline-flex items-center gap-1.5 text-[11px] text-text-muted">
          <span className="w-1.5 h-1.5 rounded-full bg-background-warning" />
          llama-server binary not found
        </span>
      );
    }
    if (isReadyForSelected) {
      return (
        <span className="inline-flex items-center gap-1.5 text-[11px] text-text-muted">
          <span className="w-1.5 h-1.5 rounded-full bg-background-success" />
          Running · {sidecar.model} ready
        </span>
      );
    }
    if (isRunningForSelected) {
      return (
        <span className="inline-flex items-center gap-1.5 text-[11px] text-text-muted">
          <span className="w-1.5 h-1.5 rounded-full bg-background-warning" />
          Running · warm-up needed
        </span>
      );
    }
    return (
      <span className="inline-flex items-center gap-1.5 text-[11px] text-text-muted">
        <span className="w-1.5 h-1.5 rounded-full bg-background-success" />
        Built in · no install needed
      </span>
    );
  })();

  return (
    <>
      <OnboardingCardShell
        chrome={chrome}
        titleId="llamacpp-setup-title"
        category="local"
        label="Local · Run on your computer"
        title="Llama Server"
        description="Pick a built-in local model and start chatting in minutes. Free, private, offline, nothing else to install."
      >
        {isChecking ? (
          <div className="flex items-center gap-2 text-xs text-text-muted">
            <div className="w-3 h-3 border-2 border-current border-t-transparent rounded-full animate-spin flex-shrink-0" />
            <span>Checking Llama Server…</span>
          </div>
        ) : binaryMissing ? (
          <div className="space-y-3">
            <div>{statusPill}</div>
            <p className="text-xs text-text-muted">
              The bundled llama-server binary is missing (development build?). Install llama.cpp
              (e.g. <code>brew install llama.cpp</code>) or set <code>BIOROUTER_LLAMACPP_BIN</code>.
            </p>
          </div>
        ) : (
          <div className="space-y-3">
            <div>{statusPill}</div>

            <div className="min-w-0">
              <label
                htmlFor="llamacpp-model-select"
                className="mb-1.5 block text-xs font-medium text-text-default"
              >
                Model
              </label>
              <select
                id="llamacpp-model-select"
                value={selectedModel}
                onChange={(e) => setSelectedModel(e.target.value)}
                disabled={isStarting || isConnecting}
                data-testid="llamacpp-model-select"
                className="block h-9 w-full min-w-0 max-w-full rounded-md border border-border-subtle bg-background-default px-2 text-sm text-text-default transition-colors duration-150 focus:border-border-strong"
              >
                {catalog.map((m) => (
                  <option key={m.name} value={m.name}>
                    {[m.display_name, m.download_size, modelDownloadLabel(m)]
                      .filter(Boolean)
                      .join(' · ')}
                  </option>
                ))}
              </select>
            </div>
            {selectedEntry && (
              <div className="min-w-0 space-y-1.5 rounded-lg border border-border-subtle bg-background-muted p-3">
                <p className="break-words text-[11px] text-text-muted">
                  {selectedEntry.description}
                </p>
                <p
                  className="break-words text-[11px] text-text-muted"
                  data-testid="llamacpp-size-speed"
                >
                  {selectedEntry.download_size} download · {selectedEntry.speed_hint}
                </p>
                <p className="break-words text-[11px] text-text-muted">
                  {modelDownloadLabel(selectedEntry)} · {modelDownloadNote(selectedEntry)}
                </p>
                {fallbackDownloadLabel(selectedEntry) && (
                  <p className="break-all text-[11px] text-text-muted">
                    Llama Server fallback: {fallbackDownloadLabel(selectedEntry)} ·{' '}
                    {selectedEntry.hf_spec}
                  </p>
                )}
                <p className="break-all text-[11px] text-text-muted">
                  {selectedEntry.ollama_name
                    ? `Ollama model: ${selectedEntry.ollama_name}`
                    : `Fallback model: ${selectedEntry.hf_spec}`}
                </p>
                {selectedEntry.model_path && (
                  <p className="truncate font-mono text-[11px] text-text-muted">
                    {selectedEntry.model_path}
                  </p>
                )}
                {system?.model_cache_dir && (
                  <p className="truncate font-mono text-[11px] text-text-muted">
                    Store: {system.model_cache_dir}
                  </p>
                )}
                {selectedEntry.suitability_message && (
                  <p className="break-words text-[11px] text-text-muted">
                    {selectedEntry.suitability_message}
                  </p>
                )}
              </div>
            )}
            {resourceWarnings.length > 0 && (
              <div className="space-y-1 rounded-md border border-border-warning bg-background-warning/10 p-2 text-[11px] text-text-default">
                {resourceWarnings.map((warning) => (
                  <p key={warning}>{warning}</p>
                ))}
              </div>
            )}

            {operation && (
              <div
                className="rounded-md border border-border-subtle bg-background-default p-3"
                data-testid="llamacpp-progress"
              >
                <div className="flex items-center gap-2 text-xs text-text-default">
                  <div className="w-3 h-3 border-2 border-current border-t-transparent rounded-full animate-spin flex-shrink-0" />
                  <span>
                    {sidecar?.state === 'ready' && sidecar.model === operation.model
                      ? `Running warm-up prompt for ${operation.model}...`
                      : sidecar?.state === 'starting'
                        ? `Preparing ${operation.model}. Loading or downloading on first use…`
                        : `Starting llama-server…`}
                  </span>
                </div>
                {(operation.message ?? sidecar?.detail) && (
                  <p className="text-[11px] text-text-muted mt-1 font-mono truncate">
                    {operation.message ?? sidecar?.detail}
                  </p>
                )}
              </div>
            )}

            <div className="flex flex-col items-stretch gap-2 pt-1 sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-4 sm:gap-y-3">
              {isReadyForSelected ? (
                <Button
                  onClick={() => connect(selectedModel)}
                  disabled={isConnecting}
                  className="h-9 w-full px-4 sm:w-auto"
                  data-testid="llamacpp-connect"
                >
                  {isConnecting ? 'Connecting…' : 'Use Llama Server'}
                </Button>
              ) : (
                <Button
                  onClick={() => handleStart(selectedModel)}
                  disabled={isStarting || isConnecting || !selectedModel}
                  className="h-9 w-full px-4 sm:w-auto"
                  data-testid="llamacpp-start"
                >
                  {startButtonLabel}
                </Button>
              )}
            </div>
          </div>
        )}
      </OnboardingCardShell>
      <ConfirmationModal
        isOpen={pendingModelStart !== null}
        title={`Load ${selectedEntry?.display_name ?? 'this model'} anyway?`}
        message={modelMemoryWarning ?? 'This model may exceed the available GPU memory.'}
        confirmLabel="Load model"
        cancelLabel="Choose another model"
        onConfirm={() => {
          const model = pendingModelStart;
          setPendingModelStart(null);
          if (model) void startModel(model);
        }}
        onCancel={() => setPendingModelStart(null)}
      />
    </>
  );
}
