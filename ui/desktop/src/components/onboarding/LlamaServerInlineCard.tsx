import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useConfig } from '../ConfigContext';
import { toastService } from '../../toasts';
import { errorMessage } from '../../utils/conversionUtils';
import { Button } from '../ui/button';
import { Select } from '../ui/Select';
import { llamacppEnsure, llamacppWarmup, type LlamaCppModel } from '../../api';
import { llamaServerStore, useLlamaServer } from '../settings/models/llamaServerStore';
import { Disclosure } from '../ui/disclosure';
import { Field } from '../ui/field';
import { localModelsCopy } from '../settings/models/copy';
import { factsLine } from '../settings/models/localModelFacts';
import { llamaCopy } from './copy';
import './onboarding.css';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { Spinner } from '../ui/spinner';
import { StatusPill } from './StatusPill';

interface LlamaServerInlineCardProps {
  onSuccess: () => void;
}

type ModelOption = { value: string; label: string };

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
      return 'ready';
    case 'partial':
      return 'partial';
    case 'not_downloaded':
      return model?.ollama_name ? 'may download' : null;
    default:
      return null;
  }
};

/**
 * One row of the Details list: a sans label and its value. A machine value (a model id, a Hugging
 * Face spec, a path) is set in mono at the supporting size, as the Settings info dialog sets them;
 * everything else is the sans face (principle 3, vision FR-R1-3: the label "Store:" was mono too).
 */
function DetailRow({
  label,
  machine = false,
  children,
}: {
  label: string;
  machine?: boolean;
  children: React.ReactNode;
}) {
  return (
    <>
      <dt className="text-supporting text-text-muted">{label}</dt>
      <dd
        className={`min-w-0 text-supporting text-text-default ${
          machine ? 'font-mono [overflow-wrap:anywhere]' : 'break-words'
        }`}
      >
        {children}
      </dd>
    </>
  );
}

export default function LlamaServerInlineCard({ onSuccess }: LlamaServerInlineCardProps) {
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
          title: llamaCopy.ready,
          msg: llamaCopy.readyDetail(model),
        });
        onSuccess();
      } catch (error) {
        connectingRef.current = false;
        if (mountedRef.current) setIsConnecting(false);
        toastService.error({
          title: 'Connection failed',
          msg: `Failed to configure Llama Server: ${errorMessage(error)}`,
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
      msg: errorMessage(error),
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
  const modelOptions = useMemo<ModelOption[]>(
    () =>
      catalog.map((m) => ({
        value: m.name,
        label: [m.display_name, m.download_size, modelDownloadLabel(m)].filter(Boolean).join(' · '),
      })),
    [catalog]
  );
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

  /**
   * What the Details list says about this machine's memory. Only a real shortfall (or a reading the
   * daemon could not take) is said, and the confirmation before a start says it again at the moment
   * of decision. The panel used to add "This model is above the 16 GB laptop tier" whenever a model
   * recommended more than 16 GB, so the model the app recommends for a 128 GiB machine opened with a
   * warning under a line saying the machine meets its recommendation (vision FR-R1-4).
   */
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
  // One verb and the size it will download (FR-S-8). Whether a fallback GGUF may download too is
  // the Details list's to say, not the button's.
  const startButtonLabel = (() => {
    if (isStarting) return llamaCopy.settingUp;
    if (isRunningForSelected) return llamaCopy.warmUp;
    if (selectedEntry?.download_status === 'downloaded') return llamaCopy.start;
    if (selectedEntry?.download_status === 'partial') {
      return llamaCopy.resumeAndStart(selectedEntry.download_size);
    }
    return llamaCopy.downloadAndStart(selectedEntry?.download_size);
  })();

  // A status only when it carries news: a server that is running, or one that cannot start. At rest
  // the row's own description already says the server is built in (FR-S-2).
  const statusPill = (() => {
    if (isChecking || !sidecar) return null;
    if (binaryMissing) return <StatusPill tone="warning">{llamaCopy.binaryMissing}</StatusPill>;
    if (isReadyForSelected) return <StatusPill tone="success">{llamaCopy.running}</StatusPill>;
    if (isRunningForSelected) {
      return <StatusPill tone="warning">{llamaCopy.warmUpNeeded}</StatusPill>;
    }
    return null;
  })();

  const largeWarning =
    selectedEntry?.suitability_status === 'above_recommendation'
      ? system?.os === 'macos'
        ? localModelsCopy.largeForThisMac
        : localModelsCopy.largeForThisComputer
      : null;
  const detail = localModelsCopy.detail;
  const fallbackState = fallbackDownloadLabel(selectedEntry);

  return (
    <>
      <div className="min-w-0">
        {isChecking ? (
          <div className="flex items-center gap-2 text-supporting text-text-muted">
            <Spinner size={14} />
            <span>{llamaCopy.checking}</span>
          </div>
        ) : binaryMissing ? (
          <div className="space-y-3">
            <div>{statusPill}</div>
            <p className="text-supporting text-text-muted">
              The bundled llama-server binary is missing (development build?). Install llama.cpp
              (e.g. <code>brew install llama.cpp</code>) or set <code>BIOROUTER_LLAMACPP_BIN</code>.
            </p>
          </div>
        ) : (
          <div className="space-y-3">
            {statusPill ? <div>{statusPill}</div> : null}

            {/* The one select (`ui/Select`), not a native <select>: the OS chevron and the OS
                popup were the only ones in the app (vision round 4, V4-R4-3). react-select names
                its own input, so the label points at `inputId`, and the test id sits on a wrapper
                (with its own id, so the Field does not give it the input's) because the library
                forwards no `data-*` attribute. The menu is portalled so a scrolling page can never
                clip it. */}
            <Field id="llamacpp-model-select" label={llamaCopy.model}>
              <div id="llamacpp-model-field" data-testid="llamacpp-model-select">
                <Select
                  inputId="llamacpp-model-select"
                  options={modelOptions}
                  value={modelOptions.find((option) => option.value === selectedModel) ?? null}
                  onChange={(option: unknown) => {
                    const chosen = option as ModelOption | null;
                    if (chosen) setSelectedModel(chosen.value);
                  }}
                  isDisabled={isStarting || isConnecting}
                  isSearchable={false}
                  menuPlacement="auto"
                  menuPortalTarget={typeof document === 'undefined' ? undefined : document.body}
                />
              </div>
            </Field>

            {/* One facts line, the Settings row's (speed · size · context, and "Large for this
                Mac" only when it is), as the closed Details' summary; the ids, paths and the fit
                sentence are behind it (vision FR-R1-3, FR-S-2). It used to be a box of seven
                lines and a warning, and pushed the panel's only action below the fold. */}
            {selectedEntry && (
              <Disclosure
                label={llamaCopy.details}
                summary={
                  <span data-testid="llamacpp-size-speed">
                    {factsLine(selectedEntry)}
                    {largeWarning ? (
                      <>
                        {' · '}
                        <span className="text-text-warning">{largeWarning}</span>
                      </>
                    ) : null}
                  </span>
                }
              >
                <div className="br-llama-details" data-testid="llamacpp-details">
                  <p className="text-supporting text-text-muted">{selectedEntry.description}</p>
                  <dl className="br-llama-details-list">
                    <DetailRow label={llamaCopy.download}>
                      {modelDownloadNote(selectedEntry)}
                    </DetailRow>
                    <DetailRow label={detail.expectedSpeed}>{selectedEntry.speed_hint}</DetailRow>
                    {selectedEntry.suitability_message ? (
                      <DetailRow label={detail.recommendation}>
                        {selectedEntry.suitability_message}
                      </DetailRow>
                    ) : null}
                    {resourceWarnings.map((warning) => (
                      <DetailRow key={warning} label={detail.detectedMemory}>
                        {warning}
                      </DetailRow>
                    ))}
                    {selectedEntry.ollama_name ? (
                      <DetailRow label={detail.ollamaModel} machine>
                        {selectedEntry.ollama_name}
                      </DetailRow>
                    ) : null}
                    <DetailRow
                      label={
                        fallbackState
                          ? `${detail.fallbackGguf}, ${fallbackState}`
                          : detail.fallbackGguf
                      }
                      machine
                    >
                      {selectedEntry.hf_spec}
                    </DetailRow>
                    {selectedEntry.model_path ? (
                      <DetailRow label={detail.modelBlob} machine>
                        {selectedEntry.model_path}
                      </DetailRow>
                    ) : null}
                    {system?.model_cache_dir ? (
                      <DetailRow label={detail.modelStore} machine>
                        {system.model_cache_dir}
                      </DetailRow>
                    ) : null}
                  </dl>
                </div>
              </Disclosure>
            )}

            {operation && (
              <div className="br-first-run-box br-fill-surface p-3" data-testid="llamacpp-progress">
                <div className="flex items-center gap-2 text-supporting text-text-default">
                  <Spinner size={14} />
                  <span>
                    {sidecar?.state === 'ready' && sidecar.model === operation.model
                      ? `Running warm-up prompt for ${operation.model}…`
                      : sidecar?.state === 'starting'
                        ? `Preparing ${operation.model}. Loading or downloading on first use…`
                        : `Starting llama-server…`}
                  </span>
                </div>
                {(operation.message ?? sidecar?.detail) && (
                  <p className="mt-1 truncate text-supporting text-text-muted">
                    {operation.message ?? sidecar?.detail}
                  </p>
                )}
              </div>
            )}

            <div className="flex flex-wrap items-center gap-2">
              {isReadyForSelected ? (
                <Button
                  onClick={() => connect(selectedModel)}
                  disabled={isConnecting}
                  data-testid="llamacpp-connect"
                >
                  {isConnecting ? llamaCopy.connecting : llamaCopy.use}
                </Button>
              ) : (
                <Button
                  onClick={() => handleStart(selectedModel)}
                  disabled={isStarting || isConnecting || !selectedModel}
                  data-testid="llamacpp-start"
                >
                  {startButtonLabel}
                </Button>
              )}
            </div>
          </div>
        )}
      </div>
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
