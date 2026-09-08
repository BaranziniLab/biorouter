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
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import { MODAL_SIZE } from '../../ModalShell';
import { Note } from '../../ui/note';
import { Skeleton } from '../../ui/skeleton';
import { cn } from '../../../utils';
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
    <div className="grid grid-cols-[minmax(8rem,auto)_minmax(0,1fr)] gap-3 text-supporting">
      <span className="text-text-muted">{label}</span>
      <span
        className={[
          'min-w-0 text-right text-text-default',
          mono ? 'break-all font-mono text-supporting' : 'break-words',
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
  // Two confirmations that used to be `window.confirm`. The OS dialog is
  // theme-blind, unstyleable and modal to the whole app, and it is the one
  // control in Settings that could not be read in dark mode. `ConfirmationModal`
  // is the app's own primitive; the resource check keeps its RESOLVER here
  // because `runInstall`/`runWarmup` consume it as a boolean guard clause and
  // must now await the user rather than a synchronous return.
  const [pendingResourceConfirm, setPendingResourceConfirm] = useState<{
    message: string;
    confirmLabel: string;
    resolve: (proceed: boolean) => void;
  } | null>(null);
  const [pendingDelete, setPendingDelete] = useState<LlamaCppModel | null>(null);
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
    (model: LlamaCppModel, confirmLabel: string) =>
      new Promise<boolean>((resolve) => {
        const system = snapshot?.system;
        if (!system || model.suitability_status === 'suitable') {
          resolve(true);
          return;
        }

        const detected =
          typeof system.accelerator_memory_gib === 'number'
            ? `${system.accelerator_memory_gib} GiB ${acceleratorMemoryLabel(
                system.accelerator_memory_kind
              )}`
            : `unknown ${acceleratorMemoryLabel(system.accelerator_memory_kind)}`;
        setPendingResourceConfirm({
          confirmLabel,
          // The same three sentences the OS dialog carried, in the same order.
          message: `${model.display_name} recommends ${model.recommended_gpu_memory_gib} GiB GPU-addressable memory. This machine reports ${detected}. ${acceleratorMemoryExplanation(system.accelerator_memory_kind)}`,
          resolve,
        });
      }),
    [snapshot?.system]
  );

  const runInstall = useCallback(
    async (model: LlamaCppModel) => {
      if (llamaServerStore.getSnapshot().operation || deleteAction) return;
      if (!(await confirmResources(model, 'Install anyway'))) return;
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
      if (!(await confirmResources(model, 'Warm up anyway'))) return;
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
      // Every one of these carries a LABEL, so none of them may use the 24px
      // `xs` rung — `--control-compact`'s own comment says it is for a
      // glyph-only control in an already-dense cluster. Dropping it also
      // retires the `h-3 w-3` overrides on their glyphs (including the three
      // spinners): the cva base supplies 16px at the default rung.
      <div className="flex shrink-0 flex-wrap items-center justify-end gap-2">
        <Button type="button" size="sm" variant="outline" onClick={() => setSelectedModel(model)}>
          <Eye />
          View info
        </Button>
        {isInstalled(model) ? (
          <>
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => void runWarmup(model)}
              disabled={busy}
            >
              {isWarming ? <Loader2 className="animate-spin" /> : <Play />}
              Warm up
            </Button>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              className="text-text-danger"
              onClick={() => setPendingDelete(model)}
              disabled={busy}
              title="Delete local model"
            >
              {isDeleting ? <Loader2 className="animate-spin" /> : <Trash2 />}
              Delete
            </Button>
          </>
        ) : (
          // `outline`, matching View info and Warm up. The status chip beside
          // the name already carries install state, and seven stacked coral
          // CTAs down one list is exactly what P3 forbids.
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() => void runInstall(model)}
            disabled={busy}
          >
            {isInstalling ? <Loader2 className="animate-spin" /> : <Download />}
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
          <p className="mt-1 text-supporting text-text-muted">
            {isLoading
              ? 'Checking local models...'
              : `${installedCount} installed · ${catalog.length} available`}
          </p>
        </div>
        {/* `mr-3` so the button's BOX shares the rows' 12px inset while the
            `text-caps` label opposite it stays flush with every other section
            header on the page. */}
        <Button
          type="button"
          shape="round"
          variant="ghost"
          className="mr-3"
          onClick={() => void refresh()}
          disabled={isLoading || busy}
          title="Refresh local model inventory"
        >
          <RefreshCw className={isLoading ? 'h-4 w-4 animate-spin' : 'h-4 w-4'} />
        </Button>
      </div>

      <div className="biorouter-settings-list">
        {error && (
          <Note
            tone="warning"
            role="alert"
            icon={AlertTriangle}
            className="mb-2"
            action={
              <Button type="button" size="sm" variant="outline" onClick={() => void refresh()}>
                Retry
              </Button>
            }
          >
            {error}
          </Note>
        )}

        {isLoading ? (
          <div className="biorouter-settings-row px-3 py-2.5">
            <Skeleton className="h-4 w-56" />
            <Skeleton className="mt-2 h-3 w-80" />
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
              <div key={model.name} className="biorouter-settings-row px-3 py-2.5">
                <div className="flex min-w-0 flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
                  <div className="min-w-0 flex-1">
                    <div className="flex min-w-0 flex-wrap items-center gap-2">
                      <p className="min-w-0 truncate text-label text-text-default">
                        {model.display_name}
                      </p>
                      <Badge tone={isInstalled(model) ? 'success' : 'neutral'}>
                        {installedLabel(model)}
                      </Badge>
                      {/* ⚠ The tone stays CONDITIONAL. An unconditional
                          `neutral` here would erase the amber "above this Mac's
                          recommendation" signal, which is the one thing on the
                          row that qualifies the Install action beside it. */}
                      <Badge tone={model.suitability_status === 'suitable' ? 'neutral' : 'warning'}>
                        {fitLabel(model)}
                      </Badge>
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
                    <p className="mt-1 text-supporting text-text-muted">
                      {model.family} · {model.download_size} · {model.speed_hint} ·{' '}
                      {formatContext(model.context_limit)} context ·{' '}
                      {model.ollama_name ?? model.hf_spec}
                    </p>
                  </div>
                  {renderAction(model)}
                </div>
                {/* No box. A transient progress line inside a row is the same
                    object as the header's own status line above it, and a
                    bordered card on a filled row was a third ground in one
                    section. */}
                {busyLabel && (
                  <div className="mt-2 flex min-w-0 items-start gap-2 text-supporting text-text-muted">
                    <Loader2 className="mt-0.5 h-4 w-4 shrink-0 animate-spin" />
                    <p className="min-w-0 whitespace-normal break-words">{busyLabel}</p>
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>

      <Dialog open={!!selectedModel} onOpenChange={(open) => !open && setSelectedModel(null)}>
        {selectedModel && (
          <DialogContent className={cn('w-[calc(100vw-2rem)] overflow-hidden', MODAL_SIZE.lg)}>
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

      {/* Not `destructive`: an over-provisioned install is a slow download, not
          a deletion, and the loud variant would say the wrong thing about it. */}
      <ConfirmationModal
        isOpen={pendingResourceConfirm !== null}
        title="Continue anyway?"
        message={pendingResourceConfirm?.message ?? ''}
        confirmLabel={pendingResourceConfirm?.confirmLabel ?? 'Continue'}
        cancelLabel="Cancel"
        confirmVariant="default"
        onConfirm={() => {
          pendingResourceConfirm?.resolve(true);
          setPendingResourceConfirm(null);
        }}
        onCancel={() => {
          pendingResourceConfirm?.resolve(false);
          setPendingResourceConfirm(null);
        }}
      />

      {/* `deleteAction` is the IN-FLIGHT state, not the pending one — hence the
          separate `pendingDelete`. Feeding `deleteAction` in as `isOpen` would
          reopen the dialog for the duration of the delete it just started. */}
      <ConfirmationModal
        isOpen={pendingDelete !== null}
        title="Delete this local model?"
        message={
          pendingDelete
            ? `${pendingDelete.ollama_name ?? pendingDelete.name} will be removed from the local model inventory. You can download it again later.`
            : ''
        }
        confirmLabel="Delete"
        cancelLabel="Cancel"
        confirmVariant="destructive"
        isSubmitting={deleteAction !== null}
        onConfirm={() => {
          const model = pendingDelete;
          setPendingDelete(null);
          if (model) void runDelete(model);
        }}
        onCancel={() => setPendingDelete(null)}
      />
    </div>
  );
}
