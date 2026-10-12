import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { llamacppDelete, llamacppEnsure, llamacppWarmup, type LlamaCppModel } from '../../../api';
import { toastService } from '../../../toasts';
import { errorMessage } from '../../../utils/conversionUtils';
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
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import { ModalShell } from '../../ModalShell';
import { Note } from '../../ui/note';
import { RowActions, RowContextMenu, type RowActionItem } from '../../ui/row-actions';
import { SettingSection } from '../../ui/setting-row';
import { Skeleton } from '../../ui/skeleton';
import { Spinner } from '../../ui/spinner';
import { StatusDot } from '../../ui/status-dot';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { AlertTriangle, ExternalLink, RefreshCw } from '../../icons/app-icons';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
import { localModelsCopy } from './copy';
import { factsLine, fitVerdict, isInstalled, stateWord } from './localModelFacts';
import './models.css';

const formatContext = (value: number | null | undefined) =>
  typeof value === 'number' ? value.toLocaleString() : 'unknown';

const acceleratorMemoryLabel = (kind: string | undefined) =>
  kind === 'apple_unified' ? 'unified memory' : 'VRAM';

const acceleratorMemoryExplanation = (kind: string | undefined) =>
  kind === 'apple_unified'
    ? 'On Apple Silicon, unified memory is the relevant GPU memory budget.'
    : 'On Intel Macs, Windows, and other discrete-GPU systems, this means VRAM, not regular system RAM.';

const progressLabel = (progress: PullProgress) => {
  if (progress.total && progress.completed) {
    const pct = Math.round((progress.completed / progress.total) * 100);
    return `${progress.status} ${pct}%`;
  }
  return progress.status;
};

const operationFallbackLabel = (operation: LlamaServerOperation) =>
  operation.kind === 'warmup' ? `Warming up ${operation.model}…` : `Preparing ${operation.model}…`;

const terminalErrorTitle = (kind: 'install' | 'start' | 'warmup') =>
  kind === 'warmup'
    ? 'Local model warm-up failed'
    : kind === 'install'
      ? 'Local model install failed'
      : 'Could not start Llama Server';

/**
 * One row of the info dialog's fact list. A MACHINE value (a model id, a Hugging Face spec, a path)
 * is set in one face for all of them: mono at the supporting size, the scale's mono size for IDs
 * (spec 5.3), so a path reads like the id above it (vision round 2, V4-R2-8). It was mono for the
 * paths alone, at the 13px code role, while the two ids beside it were sans, and one list used two
 * families for one kind of string. Prose values (a size, a speed, a sentence) stay in the body face.
 * The grid aligns on the first baseline, because the value's 12px mono and the label's 13px sans do
 * not share a line height.
 */
function DetailRow({
  label,
  children,
  machine = false,
}: {
  label: string;
  children: React.ReactNode;
  machine?: boolean;
}) {
  return (
    <div className="grid grid-cols-[minmax(8rem,auto)_minmax(0,1fr)] items-baseline gap-3 text-secondary">
      <span className="text-text-muted">{label}</span>
      <span
        className={[
          'min-w-0 text-right text-text-default',
          machine ? 'break-all font-mono text-supporting' : 'break-words',
        ]
          .filter(Boolean)
          .join(' ')}
      >
        {children}
      </span>
    </div>
  );
}

/**
 * The info dialog's body: a container, not a control. The dialog places focus on it when it opens
 * (`onOpenAutoFocus` below, V4-04) instead of on its first control, the Official URL, which wore
 * the keyboard ring after a pointer pick of View info from the right-click menu: Chromium gives
 * `:focus-visible` to every script focus on that path (the dialog box, the Close button and a bare
 * div all match), so the only way to show no ring is to focus something that draws none.
 * `.br-local-info-body` has no outline, and the first Tab reaches the URL.
 */
function InfoDialogBody({
  children,
  bodyRef,
}: {
  children: React.ReactNode;
  bodyRef: React.RefObject<HTMLDivElement | null>;
}) {
  return (
    <div ref={bodyRef} tabIndex={-1} className="br-local-info-body space-y-2 py-3">
      {children}
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

  // The info dialog places focus on its own body when it opens (`onOpenAutoFocus` on the dialog).
  // Where focus goes when it closes is the shared dialog's job: a keyboard pick gives focus back
  // to the control that opened the menu, a pointer pick gives nothing back (V4-03).
  const infoBodyRef = useRef<HTMLDivElement>(null);

  const busy = !!operation || !!deleteAction;

  const refresh = useCallback(async () => {
    try {
      if (mountedRef.current) setError(null);
      await llamaServerStore.refresh();
    } catch (err) {
      if (mountedRef.current) setError(errorMessage(err));
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
      if (!(await confirmResources(model, localModelsCopy.confirmResources.installAnyway))) return;
      let opId = llamaServerStore.beginOperation('install', model.name, 'Preparing install…', {
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
            llamaServerStore.setOperationMessage(opId, `Pulling ${model.ollama_name} from Ollama…`);
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
          'Starting Llama Server fallback download…'
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
            msg: errorMessage(err),
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
      if (!(await confirmResources(model, localModelsCopy.confirmResources.warmUpAnyway))) return;
      const opId = llamaServerStore.beginOperation('warmup', model.name, 'Warming up model…');

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
            msg: errorMessage(err),
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
      setDeleteAction({ model: model.name, message: 'Deleting local model…' });
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
          msg: errorMessage(err),
          traceback: err instanceof Error ? err.stack || '' : '',
        });
      } finally {
        if (mountedRef.current) setDeleteAction(null);
      }
    },
    [refresh]
  );

  const rowItems = (model: LlamaCppModel, busyNow: boolean): RowActionItem[] => {
    const items: RowActionItem[] = [
      { label: localModelsCopy.viewInfo, onSelect: () => setSelectedModel(model) },
    ];
    if (isInstalled(model)) {
      items.push(
        { label: localModelsCopy.warmUp, onSelect: () => void runWarmup(model), disabled: busyNow },
        { kind: 'separator' },
        {
          label: localModelsCopy.delete,
          onSelect: () => setPendingDelete(model),
          destructive: true,
          disabled: busyNow,
        }
      );
    }
    return items;
  };

  const detail = localModelsCopy.detail;
  const largeWarning =
    snapshot?.system.os === 'macos'
      ? localModelsCopy.largeForThisMac
      : localModelsCopy.largeForThisComputer;

  return (
    <SettingSection
      id={SETTINGS_SECTION_IDS.localModels}
      title={localModelsCopy.section}
      help={localModelsCopy.sectionHelp}
      action={
        <div className="br-local-models-header-action">
          {!isLoading && catalog.length > 0 ? (
            <span className="text-supporting tabular-nums text-text-muted">
              {localModelsCopy.installedCount(installedCount, catalog.length)}
            </span>
          ) : null}
          {/* A glyph alone: its name is also its tooltip, as every icon action's is. */}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                type="button"
                shape="round"
                size="sm"
                variant="ghost"
                className="br-local-models-refresh"
                onClick={() => void refresh()}
                disabled={isLoading || busy}
                aria-label={localModelsCopy.refresh}
              >
                {isLoading ? <Spinner /> : <RefreshCw />}
              </Button>
            </TooltipTrigger>
            <TooltipContent>{localModelsCopy.refresh}</TooltipContent>
          </Tooltip>
        </div>
      }
    >
      {error && (
        <Note
          tone="warning"
          role="alert"
          icon={AlertTriangle}
          className="mb-2"
          action={
            <Button type="button" size="sm" variant="secondary" onClick={() => void refresh()}>
              {localModelsCopy.retry}
            </Button>
          }
        >
          {error}
        </Note>
      )}

      {isLoading ? (
        <div className="biorouter-settings-row br-setting-row">
          <div className="br-setting-row-text gap-1.5">
            <Skeleton className="h-4 w-56" />
            <Skeleton className="h-3 w-40" />
          </div>
        </div>
      ) : (
        catalog.map((model) => {
          const installed = isInstalled(model);
          const mine = operation && operation.model === model.name ? operation : null;
          const busyLabel = mine
            ? compactStatusMessage(mine.message ?? operationFallbackLabel(mine))
            : deleteAction && deleteAction.model === model.name
              ? deleteAction.message
              : null;
          const isInstalling = mine?.kind === 'install' || mine?.kind === 'start';
          const items = rowItems(model, busy);

          return (
            <RowContextMenu key={model.name} items={items}>
              <div
                className="biorouter-settings-row br-setting-row"
                data-testid={`local-model-${model.name}`}
              >
                <div className="br-setting-row-text">
                  <div className="br-setting-row-head">
                    <span className="br-setting-row-label text-label text-text-default">
                      {model.display_name}
                    </span>
                  </div>
                  {/* One line under the name: the facts, or what is happening to the model
                      right now. The warning word shows only when the model is bigger than this
                      computer's memory recommendation. */}
                  {busyLabel ? (
                    <span
                      role="status"
                      className="br-local-model-busy text-supporting text-text-muted"
                    >
                      <Spinner size={14} />
                      <span className="min-w-0 [overflow-wrap:anywhere]">{busyLabel}</span>
                    </span>
                  ) : (
                    <span className="text-supporting text-text-muted">
                      {factsLine(model)}
                      {model.suitability_status === 'above_recommendation' ? (
                        <>
                          {' · '}
                          <span className="text-text-warning">{largeWarning}</span>
                        </>
                      ) : null}
                    </span>
                  )}
                </div>
                <div className="br-setting-row-control br-local-model-actions">
                  {/* The ⋯ comes first, so an Install button is the row's LAST control and ends
                      on the same right edge as Change…, Manage… and "Installed" (V4-25). The
                      cell it reserves is hidden at rest and opens on hover, to the left of
                      Install, so nothing moves when the pointer arrives. */}
                  <RowActions
                    menu={items}
                    size="sm"
                    menuLabel={localModelsCopy.more(model.display_name)}
                    // What an installed row says at rest, in the cell the ⋯ takes over on hover.
                    meta={
                      installed ? (
                        <span className="br-local-model-installed">
                          <StatusDot tone="success" />
                          {stateWord(model)}
                        </span>
                      ) : undefined
                    }
                  />
                  {!installed ? (
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      // The name follows the visible word ("Install Gemma 4 12B", then "Installing Gemma 4 12B"),
                      // so what a screen reader says always contains what the button shows.
                      aria-label={`${
                        isInstalling ? localModelsCopy.installing : localModelsCopy.install
                      } ${model.display_name}`}
                      onClick={() => void runInstall(model)}
                      disabled={busy}
                    >
                      {isInstalling ? (
                        <>
                          <Spinner />
                          {localModelsCopy.installing}
                        </>
                      ) : (
                        localModelsCopy.install
                      )}
                    </Button>
                  ) : null}
                </div>
              </div>
            </RowContextMenu>
          );
        })
      )}

      <ModalShell
        open={!!selectedModel}
        onOpenChange={(open) => !open && setSelectedModel(null)}
        size="lg"
        scrollBody
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          infoBodyRef.current?.focus({ preventScroll: true });
        }}
        title={selectedModel?.display_name ?? ''}
        subtitle={selectedModel?.description}
        footer={
          selectedModel ? (
            <>
              <Button type="button" variant="secondary" onClick={() => setSelectedModel(null)}>
                {localModelsCopy.close}
              </Button>
              {isInstalled(selectedModel) ? (
                <Button type="button" onClick={() => void runWarmup(selectedModel)} disabled={busy}>
                  {localModelsCopy.warmUpModel}
                </Button>
              ) : (
                <Button
                  type="button"
                  onClick={() => void runInstall(selectedModel)}
                  disabled={busy}
                >
                  {localModelsCopy.installModel}
                </Button>
              )}
            </>
          ) : null
        }
      >
        {selectedModel && (
          <InfoDialogBody bodyRef={infoBodyRef}>
            <DetailRow label={detail.status}>{stateWord(selectedModel)}</DetailRow>
            <DetailRow label={detail.family}>{selectedModel.family}</DetailRow>
            <DetailRow label={detail.download}>{selectedModel.download_size}</DetailRow>
            <DetailRow label={detail.expectedSpeed}>{selectedModel.speed_hint}</DetailRow>
            <DetailRow label={detail.context}>
              {formatContext(selectedModel.context_limit)} tokens
            </DetailRow>
            <DetailRow label={detail.minimumMemory}>
              {selectedModel.min_gpu_memory_gib} GiB GPU-addressable memory
            </DetailRow>
            <DetailRow label={detail.recommendedMemory}>
              {selectedModel.recommended_gpu_memory_gib} GiB GPU-addressable memory
            </DetailRow>
            <DetailRow label={detail.detectedMemory}>
              {typeof snapshot?.system.accelerator_memory_gib === 'number'
                ? `${snapshot.system.accelerator_memory_gib} GiB ${acceleratorMemoryLabel(
                    snapshot.system.accelerator_memory_kind
                  )}`
                : acceleratorMemoryLabel(snapshot?.system.accelerator_memory_kind)}
            </DetailRow>
            {/* The verdict in words; the numbers are the three memory rows above. The daemon's
                sentence repeated this label and printed `apple_unified` (vision FR-R2-6). */}
            <DetailRow label={detail.recommendation}>
              {fitVerdict(selectedModel, snapshot?.system)}
            </DetailRow>
            {/* Three machine values, one face: the model id and the GGUF spec are as much
                identifiers as the path below (V4-R2-8; this said "NOT mono" when the list row
                behind the dialog still printed them, and the row no longer does). */}
            <DetailRow label={detail.ollamaModel} machine>
              {selectedModel.ollama_name ?? 'none'}
            </DetailRow>
            <DetailRow label={detail.fallbackDownload} machine>
              {selectedModel.hf_spec}
            </DetailRow>
            <DetailRow label={detail.officialUrl}>
              <button
                type="button"
                className="inline-flex min-w-0 items-center justify-end gap-1 text-text-accent hover:underline"
                onClick={() =>
                  window.open(selectedModel.official_url, '_blank', 'noopener,noreferrer')
                }
              >
                <span className="truncate">{selectedModel.official_url}</span>
                <ExternalLink className="h-3.5 w-3.5 shrink-0" />
              </button>
            </DetailRow>
            <DetailRow label={detail.modelStore} machine>
              {snapshot?.system.model_cache_dir ?? 'unknown'}
            </DetailRow>
            {selectedModel.model_path && (
              <DetailRow label={detail.modelBlob} machine>
                {selectedModel.model_path}
              </DetailRow>
            )}
          </InfoDialogBody>
        )}
      </ModalShell>

      {/* Not `destructive`: an over-provisioned install is a slow download, not
          a deletion, and the loud variant would say the wrong thing about it. */}
      <ConfirmationModal
        isOpen={pendingResourceConfirm !== null}
        title={localModelsCopy.confirmResources.title}
        message={pendingResourceConfirm?.message ?? ''}
        confirmLabel={
          pendingResourceConfirm?.confirmLabel ?? localModelsCopy.confirmResources.fallbackConfirm
        }
        cancelLabel={localModelsCopy.confirmResources.cancel}
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
        title={localModelsCopy.confirmDelete.title}
        message={
          pendingDelete
            ? localModelsCopy.confirmDelete.message(pendingDelete.ollama_name ?? pendingDelete.name)
            : ''
        }
        confirmLabel={localModelsCopy.confirmDelete.confirm}
        cancelLabel={localModelsCopy.confirmDelete.cancel}
        confirmVariant="destructive"
        isSubmitting={deleteAction !== null}
        onConfirm={() => {
          const model = pendingDelete;
          setPendingDelete(null);
          if (model) void runDelete(model);
        }}
        onCancel={() => setPendingDelete(null)}
      />
    </SettingSection>
  );
}
