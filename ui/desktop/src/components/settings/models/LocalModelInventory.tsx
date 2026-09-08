import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { llamacppDelete, llamacppEnsure, llamacppWarmup, type LlamaCppModel } from '../../../api';
import { toastService } from '../../../toasts';
import {
  checkOllamaStatus,
  deleteOllamaModel,
  pullOllamaModel,
  type PullProgress,
} from '../../../utils/ollamaDetection';
import {
  compactStatusMessage,
  llamaServerStore,
  useLlamaServer,
  type LlamaServerOperation,
} from './llamaServerStore';
import { Button } from '../../ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../ui/dialog';
import {
  AlertTriangle,
  Download,
  ExternalLink,
  Eye,
  Loader2,
  Play,
  RefreshCw,
  Trash2,
} from '../../icons/app-icons';

const formatContext = (value: number | null | undefined) =>
  typeof value === 'number' ? value.toLocaleString() : 'unknown';

const acceleratorMemoryLabel = (kind: string | undefined) =>
  kind === 'apple_unified' ? 'unified memory' : 'VRAM';

const acceleratorMemoryExplanation = (kind: string | undefined) =>
  kind === 'apple_unified'
    ? 'On Apple Silicon, unified memory is the relevant GPU memory budget.'
    : 'On Intel Macs, Windows, and other discrete-GPU systems, this means VRAM, not regular system RAM.';

const isInstalled = (model: LlamaCppModel) =>
  model.download_status === 'downloaded' || model.fallback_download_status === 'downloaded';

const installedLabel = (model: LlamaCppModel) => {
  if (model.download_status === 'downloaded') {
    return model.download_source === 'ollama' ? 'Downloaded in Ollama' : 'Downloaded';
  }
  if (model.fallback_download_status === 'downloaded') return 'Fallback ready';
  if (model.download_status === 'partial' || model.fallback_download_status === 'partial') {
    return 'Partial download';
  }
  return 'Needs download';
};

const fitLabel = (model: LlamaCppModel) => {
  switch (model.suitability_status) {
    case 'suitable':
      return 'Recommended';
    case 'above_recommendation':
      return `${model.recommended_gpu_memory_gib} GiB GPU memory`;
    case 'unknown_resources':
      return 'VRAM unknown';
    default:
      return 'Unknown';
  }
};

const progressLabel = (progress: PullProgress) => {
  if (progress.total && progress.completed) {
    const pct = Math.round((progress.completed / progress.total) * 100);
    return `${progress.status} ${pct}%`;
  }
  return progress.status;
};

const operationFallbackLabel = (operation: LlamaServerOperation) =>
  operation.kind === 'warmup'
    ? `Warming up ${operation.model}...`
    : `Preparing ${operation.model}...`;

const terminalErrorTitle = (kind: 'install' | 'start' | 'warmup') =>
  kind === 'warmup'
    ? 'Local model warm-up failed'
    : kind === 'install'
      ? 'Local model install failed'
      : 'Could not start Llama Server';

function DetailRow({
  label,
  children,
  mono = false,
}: {
  label: string;
  children: React.ReactNode;
  mono?: boolean;
}) {
  return (
    <div className="grid grid-cols-[minmax(8rem,auto)_minmax(0,1fr)] gap-3 text-xs">
      <span className="text-text-muted">{label}</span>
      <span
        className={[
          'min-w-0 text-right text-text-default',
          mono ? 'break-all font-mono text-supporting leading-relaxed' : 'break-words',
        ]
          .filter(Boolean)
          .join(' ')}
      >
        {children}
      </span>
    </div>
  );
}

export default function LocalModelInventory() {
  // Status + any in-flight install/warm-up operation live in the shared
  // store, so progress survives unmounting this panel (issue #34) and an
  // operation started from onboarding is visible here too.
  const { status: snapshot, operation, lastError } = useLlamaServer();
  const [isLoading, setIsLoading] = useState(() => llamaServerStore.getSnapshot().status === null);
  const [error, setError] = useState<string | null>(null);
  const [selectedModel, setSelectedModel] = useState<LlamaCppModel | null>(null);
  // Deletes are quick, local HTTP calls with no polling; they keep
  // component-local busy state.
  const [deleteAction, setDeleteAction] = useState<{ model: string; message: string } | null>(null);
  // The install/warm-up/refresh flows outlive this panel by design (the
  // store owns the operation), but component-local state must never be set
  // after unmount — including refreshes performed after background installs.
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const busy = !!operation || !!deleteAction;

  const refresh = useCallback(async () => {
    try {
      if (mountedRef.current) setError(null);
      await llamaServerStore.refresh();
    } catch (err) {
      if (mountedRef.current) setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (mountedRef.current) setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Surface a polled terminal failure (sidecar error / deadline timeout)
  // immediately; claimErrorToast keeps it exactly-once across surfaces and
  // the driving flow's own catch handler.
  useEffect(() => {
    if (!lastError) return;
    if (!llamaServerStore.claimErrorToast(lastError.opId)) return;
    toastService.error({
      title: terminalErrorTitle(lastError.kind),
      msg: lastError.message,
    });
  }, [lastError]);

  const catalog = useMemo(() => snapshot?.catalog ?? [], [snapshot]);
  const installedCount = useMemo(() => catalog.filter(isInstalled).length, [catalog]);

  const confirmResources = useCallback(
    (model: LlamaCppModel) => {
      const system = snapshot?.system;
      if (!system || model.suitability_status === 'suitable') return true;

      const detected =
        typeof system.accelerator_memory_gib === 'number'
          ? `${system.accelerator_memory_gib} GiB ${acceleratorMemoryLabel(
              system.accelerator_memory_kind
            )}`
          : `unknown ${acceleratorMemoryLabel(system.accelerator_memory_kind)}`;
      return window.confirm(
        `${model.display_name} recommends ${model.recommended_gpu_memory_gib} GiB GPU-addressable memory.\n\nThis machine reports ${detected}.\n\n${acceleratorMemoryExplanation(system.accelerator_memory_kind)}\n\nContinue anyway?`
      );
    },
    [snapshot?.system]
  );

  const runInstall = useCallback(
    async (model: LlamaCppModel) => {
      if (llamaServerStore.getSnapshot().operation || deleteAction) return;
      if (!confirmResources(model)) return;
      let opId = llamaServerStore.beginOperation('install', model.name, 'Preparing install...', {
        poll: false,
      });
      try {
        if (model.ollama_name) {
          const ollama = await checkOllamaStatus();
          // The Ollama check may settle after this flow lost the operation
          // (deadline timeout, or a newer retry superseded it). A stale flow
          // must stop here: beginning the fallback operation below would
          // supersede the newer operation and its timers.
          if (llamaServerStore.getSnapshot().operation?.id !== opId) return;
          if (ollama.isRunning) {
            llamaServerStore.setOperationMessage(
              opId,
              `Pulling ${model.ollama_name} from Ollama...`
            );
            const pulled = await pullOllamaModel(model.ollama_name, (progress) => {
              llamaServerStore.setOperationMessage(opId, progressLabel(progress));
            });
            if (!pulled) throw new Error(`Ollama could not pull ${model.ollama_name}`);
            // End the operation BEFORE reporting success: a pull that
            // completes after its deadline (or after being superseded) must
            // not toast stale success, and ending first also disarms the
            // deadline so it cannot fire during the post-success refresh.
            if (!llamaServerStore.endOperation(opId)) return;
            toastService.success({
              title: 'Local model installed',
              msg: `${model.display_name} was downloaded with Ollama.`,
            });
            await refresh();
            return;
          }
        }

        // Switch to the polling operation: the store now tracks download
        // progress until ready/error/timeout, even if this panel unmounts.
        opId = llamaServerStore.beginOperation(
          'install',
          model.name,
          'Starting Llama Server fallback download...'
        );
        const res = await llamacppEnsure({ body: { model: model.name }, throwOnError: true });
        llamaServerStore.applyStatus(res.data, opId);
        await llamaServerStore.waitForReady(model.name, opId);
        toastService.success({
          title: 'Local model installed',
          msg: `${model.display_name} is ready in the Llama Server cache.`,
        });
        await refresh();
      } catch (err) {
        const terminal = llamaServerStore.getSnapshot().lastError;
        if (terminal?.opId === opId) {
          // The store already failed this operation terminally (polled
          // sidecar error / timeout); toast the retained error exactly once.
          if (llamaServerStore.claimErrorToast(opId)) {
            toastService.error({ title: terminalErrorTitle(terminal.kind), msg: terminal.message });
          }
        } else if (llamaServerStore.getSnapshot().operation?.id === opId) {
          // Still ours: a plain driving-flow failure.
          toastService.error({
            title: 'Local model install failed',
            msg: err instanceof Error ? err.message : String(err),
            traceback: err instanceof Error ? err.stack || '' : '',
          });
        }
        // Superseded by a newer operation: stay silent, it owns the UX.
      } finally {
        llamaServerStore.endOperation(opId);
      }
    },
    [confirmResources, deleteAction, refresh]
  );

  const runWarmup = useCallback(
    async (model: LlamaCppModel) => {
      if (llamaServerStore.getSnapshot().operation || deleteAction) return;
      if (!confirmResources(model)) return;
      const opId = llamaServerStore.beginOperation('warmup', model.name, 'Warming up model...');

      try {
        const res = await llamacppWarmup({ body: { model: model.name }, throwOnError: true });
        if (!res.data.output.trim()) {
          throw new Error('Llama Server returned an empty warm-up response');
        }
        // End the operation BEFORE reporting success: a warm-up that settles
        // after a terminal failure (polled sidecar error / deadline timeout)
        // or after being superseded must not toast stale success, and ending
        // first disarms the deadline so it cannot fire during the
        // post-success refresh below.
        if (!llamaServerStore.endOperation(opId)) return;
        toastService.success({
          title: 'Local model warmed up',
          msg: `${model.display_name} produced a test response.`,
        });
        await refresh();
      } catch (err) {
        const terminal = llamaServerStore.getSnapshot().lastError;
        if (terminal?.opId === opId) {
          if (llamaServerStore.claimErrorToast(opId)) {
            toastService.error({ title: terminalErrorTitle(terminal.kind), msg: terminal.message });
          }
        } else if (llamaServerStore.getSnapshot().operation?.id === opId) {
          toastService.error({
            title: 'Local model warm-up failed',
            msg: err instanceof Error ? err.message : String(err),
            traceback: err instanceof Error ? err.stack || '' : '',
          });
        }
      } finally {
        llamaServerStore.endOperation(opId);
      }
    },
    [confirmResources, deleteAction, refresh]
  );

  const runDelete = useCallback(
    async (model: LlamaCppModel) => {
      const label = model.ollama_name ?? model.name;
      if (!window.confirm(`Delete ${label} from the local model inventory?`)) return;

      setDeleteAction({ model: model.name, message: 'Deleting local model...' });
      try {
        let deletedSomething = false;
        if (model.download_source === 'ollama' && model.ollama_name) {
          const ollama = await checkOllamaStatus();
          if (!ollama.isRunning) {
            throw new Error('Ollama must be running to delete an Ollama-managed model.');
          }
          deletedSomething = await deleteOllamaModel(model.ollama_name);
          if (!deletedSomething) throw new Error(`Ollama could not delete ${model.ollama_name}`);
        }

        if (model.fallback_download_status === 'downloaded') {
          const res = await llamacppDelete({ body: { model: model.name }, throwOnError: true });
          deletedSomething = res.data.deleted_fallback_cache || deletedSomething;
          llamaServerStore.applyStatus(res.data.status);
        }

        if (!deletedSomething) {
          throw new Error('No cached local files were removed for this model.');
        }
        toastService.success({
          title: 'Local model deleted',
          msg: `${model.display_name} was removed from local storage.`,
        });
        await refresh();
      } catch (err) {
        toastService.error({
          title: 'Local model delete failed',
          msg: err instanceof Error ? err.message : String(err),
          traceback: err instanceof Error ? err.stack || '' : '',
        });
      } finally {
        if (mountedRef.current) setDeleteAction(null);
      }
    },
    [refresh]
  );

  const renderAction = (model: LlamaCppModel) => {
    const isInstalling =
      (operation?.kind === 'install' || operation?.kind === 'start') &&
      operation.model === model.name;
    const isWarming = operation?.kind === 'warmup' && operation.model === model.name;
    const isDeleting = deleteAction?.model === model.name;
    return (
      <div className="flex shrink-0 flex-wrap items-center justify-end gap-2">
        <Button
          type="button"
          size="xs"
          variant="outline"
          onClick={() => setSelectedModel(model)}
          disabled={busy}
        >
          <Eye className="h-3 w-3" />
          View Info
        </Button>
        {isInstalled(model) ? (
          <>
            <Button
              type="button"
              size="xs"
              variant="outline"
              onClick={() => void runWarmup(model)}
              disabled={busy}
            >
              {isWarming ? (
                <Loader2 className="h-3 w-3 animate-spin" />
              ) : (
                <Play className="h-3 w-3" />
              )}
              Warm up
            </Button>
            <Button
              type="button"
              size="xs"
              variant="ghost"
              className="text-text-danger hover:bg-background-danger/10 hover:text-text-danger"
              onClick={() => void runDelete(model)}
              disabled={busy}
              title="Delete local model"
            >
              {isDeleting ? (
                <Loader2 className="h-3 w-3 animate-spin" />
              ) : (
                <Trash2 className="h-3 w-3" />
              )}
              Delete
            </Button>
          </>
        ) : (
          <Button type="button" size="xs" onClick={() => void runInstall(model)} disabled={busy}>
            {isInstalling ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <Download className="h-3 w-3" />
            )}
            Install
          </Button>
        )}
      </div>
    );
  };

  return (
    <div className="biorouter-settings-section">
      <div className="biorouter-settings-section-header flex items-center justify-between gap-3">
        <div>
          <h2 className="text-caps text-text-muted">Local Model Inventory</h2>
          <p className="mt-1 text-xs text-text-muted">
            {isLoading
              ? 'Checking local models...'
              : `${installedCount} installed · ${catalog.length} available`}
          </p>
        </div>
        <Button
          type="button"
          size="xs"
          shape="round"
          variant="ghost"
          onClick={() => void refresh()}
          disabled={isLoading || busy}
          title="Refresh local model inventory"
        >
          <RefreshCw className={isLoading ? 'h-3 w-3 animate-spin' : 'h-3 w-3'} />
        </Button>
      </div>

      <div className="biorouter-settings-list">
        {error && (
          <div className="biorouter-settings-row flex items-start gap-2 px-3 py-3 text-xs text-text-warning">
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>{error}</span>
          </div>
        )}

        {isLoading ? (
          <div className="biorouter-settings-row px-3 py-3">
            <div className="h-4 w-56 animate-pulse rounded bg-background-medium" />
            <div className="mt-2 h-3 w-80 animate-pulse rounded bg-background-medium" />
          </div>
        ) : (
          catalog.map((model) => {
            const busyLabel =
              operation && operation.model === model.name
                ? compactStatusMessage(operation.message ?? operationFallbackLabel(operation))
                : deleteAction && deleteAction.model === model.name
                  ? deleteAction.message
                  : null;

            return (
              <div key={model.name} className="biorouter-settings-row px-3 py-3">
                <div className="flex min-w-0 flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
                  <div className="min-w-0 flex-1">
                    <div className="flex min-w-0 flex-wrap items-center gap-2">
                      <p className="min-w-0 truncate text-sm font-medium text-text-default">
                        {model.display_name}
                      </p>
                      <span
                        className={[
                          'rounded-element px-1.5 py-0.5 text-supporting',
                          isInstalled(model)
                            ? 'bg-background-success/15 text-text-success'
                            : 'bg-background-medium text-text-muted',
                        ].join(' ')}
                      >
                        {installedLabel(model)}
                      </span>
                      <span
                        className={[
                          'rounded-element px-1.5 py-0.5 text-supporting',
                          model.suitability_status === 'suitable'
                            ? 'bg-background-success/15 text-text-success'
                            : 'bg-background-warning/15 text-text-warning',
                        ].join(' ')}
                      >
                        {fitLabel(model)}
                      </span>
                    </div>
                    {/* ⚠ WRAPS, never truncates. This line was `truncate`,
                        which was survivable while Settings read the fluid page
                        measure and the label block was ~1000px wide. On the
                        chat measure the block is 508px (measured, 712px column
                        minus the action group and the row's own padding) and
                        the longest catalog entry needs 680 — so an ellipsis ate
                        the last two fields, which are the two that identify the
                        model: its context window and its `ollama_name`/`hf_spec`.
                        Truncation is for a NAME whose head identifies it; this
                        is a sentence whose tail does, so it wraps to a second
                        line instead. The name above still truncates, correctly:
                        the chips beside it wrap first. */}
                    <p className="mt-1 text-xs leading-5 text-text-muted">
                      {model.family} · {model.download_size} · {model.speed_hint} ·{' '}
                      {formatContext(model.context_limit)} context ·{' '}
                      {model.ollama_name ?? model.hf_spec}
                    </p>
                  </div>
                  {renderAction(model)}
                </div>
                {busyLabel && (
                  <div className="mt-3 flex min-w-0 items-start gap-2 rounded-element border border-border-default bg-background-medium/45 px-2.5 py-2 text-xs text-text-muted">
                    <Loader2 className="mt-0.5 h-3 w-3 shrink-0 animate-spin" />
                    <p className="min-w-0 whitespace-normal break-words leading-relaxed">
                      {busyLabel}
                    </p>
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>

      <Dialog open={!!selectedModel} onOpenChange={(open) => !open && setSelectedModel(null)}>
        {selectedModel && (
          <DialogContent className="w-[calc(100vw-2rem)] overflow-hidden sm:max-w-[640px]">
            <DialogHeader>
              <DialogTitle>{selectedModel.display_name}</DialogTitle>
              <DialogDescription>{selectedModel.description}</DialogDescription>
            </DialogHeader>

            <div className="max-h-[58vh] space-y-2 overflow-y-auto pr-1">
              <DetailRow label="Status">{installedLabel(selectedModel)}</DetailRow>
              <DetailRow label="Family">{selectedModel.family}</DetailRow>
              <DetailRow label="Download">{selectedModel.download_size}</DetailRow>
              <DetailRow label="Expected speed">{selectedModel.speed_hint}</DetailRow>
              <DetailRow label="Context">
                {formatContext(selectedModel.context_limit)} tokens
              </DetailRow>
              <DetailRow label="Minimum memory">
                {selectedModel.min_gpu_memory_gib} GiB GPU-addressable memory
              </DetailRow>
              <DetailRow label="Recommended memory">
                {selectedModel.recommended_gpu_memory_gib} GiB GPU-addressable memory
              </DetailRow>
              <DetailRow label="Detected memory">
                {typeof snapshot?.system.accelerator_memory_gib === 'number'
                  ? `${snapshot.system.accelerator_memory_gib} GiB ${acceleratorMemoryLabel(
                      snapshot.system.accelerator_memory_kind
                    )}`
                  : acceleratorMemoryLabel(snapshot?.system.accelerator_memory_kind)}
              </DetailRow>
              <DetailRow label="Recommendation">{selectedModel.suitability_message}</DetailRow>
              {/* NOT `mono`. These are the same two fields the list row behind
                  this dialog prints in the body font, and a model id is a NAME
                  everywhere else in the app — the composer chip, the model
                  pickers, the onboarding cards, ApplicationsView. `mono` here
                  made one field change face between the row and the dialog it
                  opens. The `mono` rows below stay: those are filesystem
                  paths, which is a job mono actually earns (D-31). */}
              <DetailRow label="Ollama model">{selectedModel.ollama_name ?? 'none'}</DetailRow>
              <DetailRow label="Fallback GGUF">{selectedModel.hf_spec}</DetailRow>
              <DetailRow label="Official URL">
                <button
                  type="button"
                  className="inline-flex min-w-0 items-center justify-end gap-1 text-text-accent hover:underline"
                  onClick={() =>
                    window.open(selectedModel.official_url, '_blank', 'noopener,noreferrer')
                  }
                >
                  <span className="truncate">{selectedModel.official_url}</span>
                  <ExternalLink className="h-3 w-3 shrink-0" />
                </button>
              </DetailRow>
              <DetailRow label="Model store" mono>
                {snapshot?.system.model_cache_dir ?? 'unknown'}
              </DetailRow>
              {selectedModel.model_path && (
                <DetailRow label="Model blob" mono>
                  {selectedModel.model_path}
                </DetailRow>
              )}
            </div>

            <DialogFooter className="flex-col gap-2 pt-2 sm:flex-row">
              <Button
                type="button"
                variant="outline"
                className="w-full sm:w-auto"
                onClick={() => setSelectedModel(null)}
              >
                Close
              </Button>
              {isInstalled(selectedModel) ? (
                <Button
                  type="button"
                  className="w-full sm:w-auto"
                  onClick={() => void runWarmup(selectedModel)}
                  disabled={busy}
                >
                  Warm up model
                </Button>
              ) : (
                <Button
                  type="button"
                  className="w-full sm:w-auto"
                  onClick={() => void runInstall(selectedModel)}
                  disabled={busy}
                >
                  Install model
                </Button>
              )}
            </DialogFooter>
          </DialogContent>
        )}
      </Dialog>
    </div>
  );
}
