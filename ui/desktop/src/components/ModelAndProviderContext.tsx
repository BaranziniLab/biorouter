import React, { createContext, useContext, useState, useEffect, useMemo, useCallback } from 'react';
import { toastError, toastSuccess } from '../toasts';
import Model, {
  getProviderMetadata,
  modelSupportedInputMimeTypes,
  modelSupportsVision,
} from './settings/models/modelInterface';
import {
  ProviderMetadata,
  setConfigProvider,
  updateAgentProvider,
  llamacppStatus,
  llamacppWarmup,
  type LlamaCppModel,
  type LlamaCppStatusResponse,
  type PrivacyBarrierBody,
} from '../api';
import { useConfig } from './ConfigContext';
import { isUserActionRefusal, userActionHeaders } from '../utils/userAction';
import { showCrossAffiliationNotice } from '../utils/crossAffiliationNotice';
import {
  getModelDisplayName,
  getProviderDisplayName,
} from './settings/models/predefinedModelsUtils';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './ui/dialog';
import { Button } from './ui/button';
import { notifySessionToolsChanged } from '../utils/sessionToolEvents';
import { announceSessionBinding } from '../utils/sessionBindingSync';

// titles
export const UNKNOWN_PROVIDER_TITLE = 'Provider name lookup';

// errors
export const UNKNOWN_PROVIDER_MSG = 'Unknown provider in config. Check your config.yaml.';

// success
const CHANGE_MODEL_TOAST_TITLE = 'Model changed';
const SWITCH_MODEL_SUCCESS_MSG = 'Switched models';

/**
 * Issue #56 DR-16. The one refusal in this feature addressed to the USER rather
 * than to the model, and one the user should ordinarily never see: the model
 * picker carries the proof, so this appears only on a backend the app did not
 * start and which was therefore handed no user-action key (open question 23).
 *
 * It names that cause instead of accusing the person at the keyboard of being a
 * model, and it says what still works — because most of the app does.
 */
export const NO_USER_PROOF_TOAST_TITLE = "Can't switch this chat to a private model";
export const NO_USER_PROOF_TOAST_MSG =
  'This chat is connected to a backend started outside the Biorouter app, which has no way to ' +
  'confirm a request came from you. Chats already on a private model keep working, and ' +
  'switching to a public model still works. To use a private model, open this chat in the ' +
  'Biorouter app.';

/**
 * Whether `currentModel`/`currentProvider` mean anything yet.
 *
 * Both start out `null`, and until the config has been read that `null` says
 * "not known", not "nothing configured". Consumers that draw a conclusion from
 * a null pair — telling the user no model is configured, sending them to
 * Settings, disabling an action — must wait for `ready`, or they will say it
 * about a perfectly valid configuration that is simply still loading.
 */
export type ModelConfigStatus = 'loading' | 'ready';

interface ModelAndProviderContextType {
  currentModel: string | null;
  currentProvider: string | null;
  modelConfigStatus: ModelConfigStatus;
  currentModelSupportsVision: boolean;
  currentModelSupportedInputMimeTypes: string[] | null;
  changeModel: (sessionId: string | null, model: Model) => Promise<boolean>;
  getCurrentModelAndProvider: () => Promise<{ model: string; provider: string }>;
  getFallbackModelAndProvider: () => Promise<{ model: string; provider: string }>;
  getCurrentModelAndProviderForDisplay: () => Promise<{ model: string; provider: string }>;
  getCurrentModelDisplayName: () => Promise<string>;
  getCurrentProviderDisplayName: () => Promise<string>; // Gets provider display name from subtext
  refreshCurrentModelAndProvider: () => Promise<void>;
}

interface ModelAndProviderProviderProps {
  children: React.ReactNode;
}

type LlamaWarmupDialogState = {
  model: Model;
  entry?: LlamaCppModel;
  status: LlamaCppStatusResponse;
  isWarming: boolean;
  detail?: string;
  resolve: (ok: boolean) => void;
};

const LOCAL_PROVIDER = 'llamacpp';
const WARMUP_POLL_INTERVAL_MS = 1500;

const formatContext = (tokens: number | undefined) =>
  typeof tokens === 'number' && tokens > 0 ? tokens.toLocaleString() : 'unknown';

const errorMessage = (error: unknown) => (error instanceof Error ? error.message : String(error));

/**
 * Issue #56 Gate A. The 409 body, if this is one.
 *
 * Under `throwOnError: true` the generated @hey-api client throws the PARSED
 * RESPONSE BODY rather than an `Error` (see `api/client/client.gen.ts`), so the
 * typed barrier arrives here verbatim. Anything else — a network failure, a 500
 * — falls through to the generic error toast.
 */
const privacyBarrierOf = (error: unknown): PrivacyBarrierBody | null =>
  error && typeof error === 'object' && (error as { code?: unknown }).code === 'privacy_barrier'
    ? (error as PrivacyBarrierBody)
    : null;

/**
 * The Gate A refusal card (design §14.4): what happened, which two tiers
 * collided, why the boundary exists, and the shortest way forward.
 *
 * It names the tier and the models only. Never the chat's title or working
 * directory — a refusal must not carry conversation content.
 */
const privacyBarrierMessage = (barrier: PrivacyBarrierBody) => {
  const lines = [
    'This chat is private, so it can only run on a private model. Biorouter will not send its contents to a model hosted outside your institution.',
  ];
  if (barrier.available_private_providers.length > 0) {
    lines.push(`Available private models: ${barrier.available_private_providers.join(', ')}.`);
  }
  lines.push(
    'To use a public model here, make this chat public first (History → this chat → Make public). That permanently exposes its contents and cannot be undone.'
  );
  return lines.join('\n');
};

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

const fallbackDownloadLabel = (model: LlamaCppModel | undefined) => {
  switch (model?.fallback_download_status) {
    case 'downloaded':
      return 'Fallback ready';
    case 'partial':
      return 'Fallback partial';
    case 'not_downloaded':
      return model?.ollama_name ? 'Fallback may download' : 'Not cached';
    default:
      return 'unknown';
  }
};

const WarmupDetailRow = ({
  label,
  children,
  mono = false,
}: {
  label: string;
  children: React.ReactNode;
  mono?: boolean;
}) => (
  <div className="grid grid-cols-[minmax(7.5rem,auto)_minmax(0,1fr)] items-start gap-x-3 gap-y-1">
    <span className="text-text-muted">{label}</span>
    <span
      className={[
        'min-w-0 text-right text-text-default',
        mono ? 'break-all font-mono text-[11px] leading-relaxed' : '',
      ]
        .filter(Boolean)
        .join(' ')}
    >
      {children}
    </span>
  </div>
);

const ModelAndProviderContext = createContext<ModelAndProviderContextType | undefined>(undefined);

export const ModelAndProviderProvider: React.FC<ModelAndProviderProviderProps> = ({ children }) => {
  const [currentModel, setCurrentModel] = useState<string | null>(null);
  const [currentProvider, setCurrentProvider] = useState<string | null>(null);
  const [modelConfigStatus, setModelConfigStatus] = useState<ModelConfigStatus>('loading');
  const [currentModelSupportsVision, setCurrentModelSupportsVision] = useState<boolean>(false);
  const [currentModelSupportedInputMimeTypes, setCurrentModelSupportedInputMimeTypes] = useState<
    string[] | null
  >(null);
  const [llamaWarmupDialog, setLlamaWarmupDialog] = useState<LlamaWarmupDialogState | null>(null);
  const { read, getProviders, refreshConfig } = useConfig();

  /**
   * Invalidate ConfigContext's cached snapshot after a write that bypassed it.
   *
   * `setConfigProvider` writes BIOROUTER_PROVIDER/BIOROUTER_MODEL straight to
   * the API, so nothing else tells that cache its copy is out of date and every
   * consumer reading those keys keeps seeing the pre-switch pair (issue #52).
   *
   * A failed refresh must not fail the switch it follows: the write already
   * landed, and reporting the model change as failed because a *cache* could
   * not be re-read would be a lie in the more alarming direction.
   */
  const refreshCachedConfig = useCallback(async () => {
    try {
      await refreshConfig();
    } catch (error) {
      console.error('Failed to refresh the cached config after a provider write:', error);
    }
  }, [refreshConfig]);

  const resolveWarmupDialog = useCallback(
    (ok: boolean) => {
      llamaWarmupDialog?.resolve(ok);
      setLlamaWarmupDialog(null);
    },
    [llamaWarmupDialog]
  );

  const prepareLlamaModel = useCallback(async (model: Model): Promise<boolean> => {
    const status = await llamacppStatus({ throwOnError: true });
    const sidecar = status.data.sidecar;
    if (sidecar.state === 'ready' && sidecar.model === model.name && sidecar.warmed) {
      return true;
    }

    return new Promise<boolean>((resolve) => {
      setLlamaWarmupDialog({
        model,
        entry: status.data.catalog.find((entry) => entry.name === model.name),
        status: status.data,
        isWarming: false,
        detail: sidecar.detail || undefined,
        resolve,
      });
    });
  }, []);

  const handleWarmupConfirm = useCallback(async () => {
    const dialog = llamaWarmupDialog;
    if (!dialog || dialog.isWarming) return;

    setLlamaWarmupDialog((current) =>
      current
        ? {
            ...current,
            isWarming: true,
            detail: 'Starting Llama Server and waiting for a test response...',
          }
        : current
    );

    let poll: number | null = window.setInterval(async () => {
      try {
        const res = await llamacppStatus({ throwOnError: true });
        setLlamaWarmupDialog((current) =>
          current?.resolve === dialog.resolve
            ? {
                ...current,
                status: res.data,
                detail:
                  res.data.sidecar.detail ||
                  (res.data.sidecar.state === 'ready'
                    ? 'Running a warm-up prompt...'
                    : 'Loading model...'),
              }
            : current
        );
      } catch {
        // Keep the primary warm-up request in charge of the final result.
      }
    }, WARMUP_POLL_INTERVAL_MS);

    try {
      const res = await llamacppWarmup({
        body: { model: dialog.model.name },
        throwOnError: true,
      });
      if (!res.data.output.trim()) {
        throw new Error('Llama Server returned an empty warm-up response');
      }
      if (poll !== null) {
        window.clearInterval(poll);
        poll = null;
      }
      dialog.resolve(true);
      setLlamaWarmupDialog(null);
    } catch (error) {
      if (poll !== null) {
        window.clearInterval(poll);
      }
      toastError({
        title: 'Llama Server warm-up failed',
        msg: errorMessage(error),
        traceback: errorMessage(error),
      });
      dialog.resolve(false);
      setLlamaWarmupDialog(null);
    }
  }, [llamaWarmupDialog]);

  const changeModel = useCallback(
    async (sessionId: string | null, model: Model) => {
      const modelName = model.name;
      const providerName = model.provider;
      let phase = 'agent';

      try {
        if (providerName === LOCAL_PROVIDER) {
          const warmed = await prepareLlamaModel(model);
          if (!warmed) {
            return false;
          }
        }

        // Issue #56 DR-26. The bind's own answer, kept so the warning it carries
        // can be shown once the switch has actually succeeded — `undefined` when
        // this call was skipped because there is no chat to bind.
        //
        // ⚠ Typed `unknown`, matching what `showCrossAffiliationNotice` takes.
        // The generated client declares this route's 200 as `unknown` (the
        // OpenAPI `body = String` this handler now carries has not been
        // regenerated into `src/api/` yet), and a narrower annotation here would
        // have to be revised the moment it is — while the runtime check inside
        // the presenter is what actually decides.
        let boundNotice: unknown;
        if (sessionId) {
          const bound = await updateAgentProvider({
            body: {
              session_id: sessionId,
              provider: providerName,
              model: modelName,
              context_limit: model.context_limit,
              request_params: model.request_params,
            },
            // Issue #56 DR-16: THIS is the model picker, so this request is the
            // user's act. Without the header the daemon cannot tell it from a
            // model curling the same route and refuses every switch to a
            // private model.
            headers: await userActionHeaders(),
            // Issue #56: without this the generated @hey-api client returns
            // {error} instead of throwing, so a 409 privacy refusal is
            // discarded, setConfigProvider rewrites the global default to the
            // refused provider, and a green toast claims the switch worked.
            throwOnError: true,
          });
          boundNotice = bound.data;
          notifySessionToolsChanged(sessionId);
          // Round 3 / N1. The row this chat runs on has just been rewritten, and
          // the renderer's copy of it is a cache nothing else refreshes — so the
          // composer, which now states the chat's own binding whenever it
          // differs from the app-wide selection, would keep naming the model the
          // user just switched away from.
          //
          // ⚠ **Here, not below.** This lands BEFORE `setConfigProvider` and
          // before `setCurrentProvider`/`setCurrentModel`, so in the only render
          // where the two can disagree the ROW holds the new binding and the
          // selection still holds the old one. Announcing after the selection
          // moved would invert that window and flash the old model.
          //
          // ⚠ And only after `updateAgentProvider` RESOLVED: a refusal (Gate A's
          // 409 for a public model on a private chat) throws past this line, and
          // the row it did not write must not be reported as written.
          announceSessionBinding({
            sessionId,
            provider: providerName,
            model: modelName,
            contextLimit: model.context_limit,
          });
        }

        phase = 'config';
        await setConfigProvider({
          body: {
            provider: providerName,
            model: modelName,
          },
          headers: await userActionHeaders(),
          throwOnError: true,
        });

        setCurrentProvider(providerName);
        setCurrentModel(modelName);
        setModelConfigStatus('ready');
        await refreshCachedConfig();

        toastSuccess({
          title: CHANGE_MODEL_TOAST_TITLE,
          msg: `${SWITCH_MODEL_SUCCESS_MSG} — using ${model.alias ?? modelName} from ${model.subtext ?? providerName}`,
        });
        // Issue #56 DR-26 at the BIND surface. Binding a model covered by one
        // institution's agreements into a chat holding another institution's
        // connectors is a mismatch the daemon has detected since Task 48 and only
        // ever logged — so the user got the success toast above and no statement
        // at all until the first tool call was refused. The body is the daemon's
        // own words, naming both institutions, and it is empty for every bind
        // that crosses no boundary, which is nearly all of them.
        //
        // ⚠ **After the success toast and after the `return`-less path, not in
        // place of them.** DR-26 warns; it does not refuse. The switch happened,
        // it is reported as having happened, and this adds what the success toast
        // cannot say. A refused switch never reaches here — it throws into the
        // `catch` below, where the Gate A card explains a boundary that BLOCKED.
        showCrossAffiliationNotice(boundNotice);
        return true;
      } catch (error) {
        console.error(`Failed to change model at ${phase} step -- ${modelName} ${providerName}`);
        // A privacy refusal is not a failure to report — it is a boundary to
        // explain. Rendered as the Gate A card rather than as a stack trace.
        const barrier = privacyBarrierOf(error);
        if (barrier) {
          toastError({
            title: `Can't switch this chat to ${model.alias ?? modelName}`,
            msg: privacyBarrierMessage(barrier),
            traceback: privacyBarrierMessage(barrier),
          });
          return false;
        }
        // Issue #56 DR-16. The daemon refused because the request carried no
        // proof it came from the user — which on this path means the backend was
        // started outside the app and has no user-action key at all, since the
        // picker always sends the header. The refusal body is model-facing
        // prose; falling through to the generic arm below would report a policy
        // refusal as a provider failure with a raw error string, which is the
        // failure mode the Gate A comment above exists to prevent.
        if (isUserActionRefusal(error)) {
          toastError({
            title: NO_USER_PROOF_TOAST_TITLE,
            msg: NO_USER_PROOF_TOAST_MSG,
            traceback: errorMessage(error),
          });
          return false;
        }
        toastError({
          title: `${providerName}/${modelName} failed`,
          msg: `${error}`,
          traceback: error instanceof Error ? error.message : String(error),
        });
        return false;
      }
    },
    [prepareLlamaModel, refreshCachedConfig]
  );

  const getFallbackModelAndProvider = useCallback(async () => {
    const provider = window.appConfig.get('BIOROUTER_DEFAULT_PROVIDER') as string;
    const model = window.appConfig.get('BIOROUTER_DEFAULT_MODEL') as string;
    if (provider && model) {
      try {
        await setConfigProvider({
          body: {
            provider: provider,
            model: model,
          },
          // Issue #56 DR-16: `/config/set_provider` writes BIOROUTER_PROVIDER
          // by construction and so is guarded unconditionally. This is the
          // app's own first-run seeding of the bundled default — still a
          // renderer act, and without the header it would 409 on every launch
          // that has no provider configured yet.
          headers: await userActionHeaders(),
          throwOnError: true,
        });
        // Same API-mediated write, same stale cache (#52).
        await refreshCachedConfig();
      } catch (error) {
        console.error('[getFallbackModelAndProvider] Failed to write to config', error);
      }
    }
    return { model: model, provider: provider };
  }, [refreshCachedConfig]);

  const getCurrentModelAndProvider = useCallback(async () => {
    let model: string;
    let provider: string;

    // read from config
    try {
      model = (await read('BIOROUTER_MODEL', false)) as string;
      provider = (await read('BIOROUTER_PROVIDER', false)) as string;
    } catch {
      console.error(`Failed to read BIOROUTER_MODEL or BIOROUTER_PROVIDER from config`);
      throw new Error('Failed to read BIOROUTER_MODEL or BIOROUTER_PROVIDER from config');
    }
    if (!model || !provider) {
      console.log('[getCurrentModelAndProvider] Checking app environment as fallback');
      return getFallbackModelAndProvider();
    }
    return { model: model, provider: provider };
  }, [read, getFallbackModelAndProvider]);

  const getCurrentModelAndProviderForDisplay = useCallback(async () => {
    const modelProvider = await getCurrentModelAndProvider();
    const model = modelProvider.model;
    const providerName = modelProvider.provider;

    // lookup display name
    let metadata: ProviderMetadata;

    try {
      metadata = await getProviderMetadata(String(providerName), getProviders);
    } catch {
      return { model: model, provider: providerName };
    }
    const providerDisplayName = metadata.display_name;

    return { model: model, provider: providerDisplayName };
  }, [getCurrentModelAndProvider, getProviders]);

  const getCurrentModelDisplayName = useCallback(async () => {
    try {
      const currentModelName = (await read('BIOROUTER_MODEL', false)) as string;
      // ⚠ `?? 'Select Model'`, and the return type was a lie without it. This is
      // declared `Promise<string>`, but `getModelDisplayName` answers `null` for
      // a model it does not recognise — including the empty one an install with
      // no provider has — and `ModelsBottomBar` stores the result and then reads
      // `.length` off it. The whole renderer crashed to the error boundary with
      // "Cannot read properties of null (reading 'length')". It was unreachable
      // only because onboarding could not be skipped; the moment a user could
      // enter the app unconfigured, it was the first thing they saw.
      return getModelDisplayName(currentModelName) ?? 'Select Model';
    } catch {
      return 'Select Model';
    }
  }, [read]);

  const getCurrentProviderDisplayName = useCallback(async () => {
    try {
      const currentModelName = (await read('BIOROUTER_MODEL', false)) as string;
      const providerDisplayName = getProviderDisplayName(currentModelName);
      if (providerDisplayName) {
        return providerDisplayName;
      }
      // Fall back to regular provider display name lookup
      const { provider } = await getCurrentModelAndProviderForDisplay();
      return provider;
    } catch {
      return '';
    }
  }, [read, getCurrentModelAndProviderForDisplay]);

  const refreshCurrentModelAndProvider = useCallback(async () => {
    try {
      const { model, provider } = await getCurrentModelAndProvider();
      setCurrentModel(model);
      setCurrentProvider(provider);
    } catch (_error) {
      console.error('Failed to refresh current model and provider:', _error);
    } finally {
      // Ready even when the read failed: a failed read is an answer ("we could
      // not find a configured model"), and leaving the status at `loading`
      // would park every consumer on a spinner that never resolves.
      setModelConfigStatus('ready');
    }
  }, [getCurrentModelAndProvider]);

  // Derive vision support whenever the active model/provider changes
  useEffect(() => {
    let cancelled = false;
    if (!currentModel || !currentProvider) {
      setCurrentModelSupportsVision(false);
      setCurrentModelSupportedInputMimeTypes(null);
      return;
    }
    (async () => {
      try {
        const metadata = await getProviderMetadata(currentProvider, getProviders);
        if (!cancelled) {
          setCurrentModelSupportsVision(modelSupportsVision(metadata, currentModel));
          setCurrentModelSupportedInputMimeTypes(
            modelSupportedInputMimeTypes(metadata, currentModel)
          );
        }
      } catch {
        if (!cancelled) {
          setCurrentModelSupportsVision(false);
          setCurrentModelSupportedInputMimeTypes(null);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [currentModel, currentProvider, getProviders]);

  // Load initial model and provider on mount
  useEffect(() => {
    refreshCurrentModelAndProvider();
  }, [refreshCurrentModelAndProvider]);

  const llamaWarnings = useMemo(() => {
    if (!llamaWarmupDialog) return [];

    const warnings: string[] = [];
    const entry = llamaWarmupDialog.entry;
    const system = llamaWarmupDialog.status.system;
    const memory = system.accelerator_memory_gib;
    const memoryLabel = acceleratorMemoryLabel(system.accelerator_memory_kind);

    if (entry && typeof memory === 'number' && memory < entry.recommended_gpu_memory_gib) {
      warnings.push(
        `This machine reports ${memory} GiB ${memoryLabel}; ${entry.display_name} recommends ${entry.recommended_gpu_memory_gib} GiB GPU-addressable memory. ${acceleratorMemoryExplanation(system.accelerator_memory_kind)}`
      );
    } else if (!entry) {
      warnings.push(
        'Custom Hugging Face specs are not memory-rated here. Start with a small quantization or lower LLAMACPP_CONTEXT_SIZE on laptop hardware.'
      );
    } else if (entry && memory == null) {
      warnings.push(
        `Biorouter could not detect VRAM. ${entry.display_name} recommends ${entry.recommended_gpu_memory_gib} GiB GPU-addressable memory. ${acceleratorMemoryExplanation(system.accelerator_memory_kind)}`
      );
    }

    if (entry && entry.recommended_gpu_memory_gib > 16) {
      warnings.push(
        `${entry.display_name} is above the 16 GB laptop tier. On 16 GB machines, use Gemma 4 unless the app reports enough GPU-addressable memory.`
      );
    }

    if (system.os.toLowerCase().includes('windows')) {
      warnings.push(
        'On Windows, make sure free VRAM is high enough for the model and context window; regular system RAM does not satisfy the GPU memory recommendation.'
      );
    }

    return warnings;
  }, [llamaWarmupDialog]);

  const contextValue = useMemo(
    () => ({
      currentModel,
      currentProvider,
      modelConfigStatus,
      currentModelSupportsVision,
      currentModelSupportedInputMimeTypes,
      changeModel,
      getCurrentModelAndProvider,
      getFallbackModelAndProvider,
      getCurrentModelAndProviderForDisplay,
      getCurrentModelDisplayName,
      getCurrentProviderDisplayName,
      refreshCurrentModelAndProvider,
    }),
    [
      currentModel,
      currentProvider,
      modelConfigStatus,
      currentModelSupportsVision,
      currentModelSupportedInputMimeTypes,
      changeModel,
      getCurrentModelAndProvider,
      getFallbackModelAndProvider,
      getCurrentModelAndProviderForDisplay,
      getCurrentModelDisplayName,
      getCurrentProviderDisplayName,
      refreshCurrentModelAndProvider,
    ]
  );

  return (
    <>
      <ModelAndProviderContext.Provider value={contextValue}>
        {children}
      </ModelAndProviderContext.Provider>

      <Dialog
        open={!!llamaWarmupDialog}
        onOpenChange={(open) => {
          if (!open && !llamaWarmupDialog?.isWarming) {
            resolveWarmupDialog(false);
          }
        }}
      >
        {/*
          `max-h` + a scrolling BODY, not just `overflow-hidden`. This sheet's detail
          block grows with the model (blob path, suitability message, fallback state),
          and with no height ceiling the dialog centred itself taller than the viewport
          — putting its own "Warm up" and "Keep previous" buttons off-screen, where they
          are unreachable rather than merely ugly. The ceiling lives here and the scroll
          lives on the body below, so the header and footer stay pinned and the footer's
          controls are always in view. Guarded by ModelAndProviderContext.warmupDialog.test.tsx.
        */}
        <DialogContent className="flex max-h-[calc(100vh-4rem)] w-[calc(100vw-2rem)] flex-col overflow-hidden sm:max-w-[520px]">
          <DialogHeader>
            <DialogTitle>Warm up local model</DialogTitle>
            <DialogDescription>
              {llamaWarmupDialog?.entry?.display_name ?? llamaWarmupDialog?.model.name} runs on this
              computer. First use can take a while because the model may need to load, download, and
              produce a test response.
            </DialogDescription>
          </DialogHeader>

          {llamaWarmupDialog && (
            <div className="min-h-0 flex-1 space-y-4 overflow-y-auto pr-1 text-sm">
              <div className="min-w-0 rounded-md border border-border-subtle bg-background-medium p-3">
                <div className="grid min-w-0 gap-1.5 text-xs">
                  <WarmupDetailRow label="Download">
                    {llamaWarmupDialog.entry?.download_size ?? 'custom'}
                  </WarmupDetailRow>
                  <WarmupDetailRow label="Local copy">
                    {modelDownloadLabel(llamaWarmupDialog.entry)}
                  </WarmupDetailRow>
                  <WarmupDetailRow label="Llama Server fallback">
                    {fallbackDownloadLabel(llamaWarmupDialog.entry)}
                  </WarmupDetailRow>
                  <WarmupDetailRow label="Ollama model">
                    {llamaWarmupDialog.entry?.ollama_name ?? 'custom'}
                  </WarmupDetailRow>
                  <WarmupDetailRow label="Model store" mono>
                    {llamaWarmupDialog.status.system.model_cache_dir}
                  </WarmupDetailRow>
                  {llamaWarmupDialog.entry?.model_path && (
                    <WarmupDetailRow label="Model blob" mono>
                      {llamaWarmupDialog.entry.model_path}
                    </WarmupDetailRow>
                  )}
                  {llamaWarmupDialog.entry?.suitability_message && (
                    <div className="pt-1 text-text-default break-words">
                      {llamaWarmupDialog.entry.suitability_message}
                    </div>
                  )}
                  <WarmupDetailRow label="Default context">
                    {formatContext(llamaWarmupDialog.status.system.default_context_size)} tokens
                  </WarmupDetailRow>
                  <WarmupDetailRow label="Detected GPU memory">
                    {typeof llamaWarmupDialog.status.system.accelerator_memory_gib === 'number'
                      ? `${llamaWarmupDialog.status.system.accelerator_memory_gib} GiB ${acceleratorMemoryLabel(llamaWarmupDialog.status.system.accelerator_memory_kind)}`
                      : acceleratorMemoryLabel(
                          llamaWarmupDialog.status.system.accelerator_memory_kind
                        )}
                  </WarmupDetailRow>
                  <WarmupDetailRow label="Recommended GPU memory">
                    {llamaWarmupDialog.entry
                      ? `${llamaWarmupDialog.entry.recommended_gpu_memory_gib} GiB`
                      : 'unknown'}
                  </WarmupDetailRow>
                </div>
              </div>

              {llamaWarnings.length > 0 && (
                <div className="space-y-2 rounded-md border border-border-warning bg-background-warning/10 p-3 text-xs text-text-default">
                  {llamaWarnings.map((warning) => (
                    <p key={warning}>{warning}</p>
                  ))}
                </div>
              )}

              {llamaWarmupDialog.isWarming && (
                <div className="flex items-start gap-2 rounded-md border border-border-subtle bg-background-default p-3 text-xs text-text-muted">
                  <div className="mt-0.5 h-3 w-3 flex-shrink-0 rounded-full border-2 border-current border-t-transparent animate-spin" />
                  <div className="min-w-0">
                    <p className="text-text-default">Waiting for the model to generate...</p>
                    {llamaWarmupDialog.detail && (
                      <p className="mt-1 truncate font-mono">{llamaWarmupDialog.detail}</p>
                    )}
                  </div>
                </div>
              )}
            </div>
          )}

          <DialogFooter className="flex-col gap-2 pt-2 sm:flex-row">
            <Button
              type="button"
              variant="outline"
              className="w-full sm:w-auto"
              onClick={() => resolveWarmupDialog(false)}
              disabled={llamaWarmupDialog?.isWarming}
            >
              Keep previous model
            </Button>
            <Button
              type="button"
              className="w-full sm:w-auto"
              onClick={handleWarmupConfirm}
              disabled={!llamaWarmupDialog || llamaWarmupDialog.isWarming}
            >
              {llamaWarmupDialog?.isWarming ? 'Warming up...' : 'Warm up model'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
};

export const useModelAndProvider = () => {
  const context = useContext(ModelAndProviderContext);
  if (context === undefined) {
    throw new Error('useModelAndProvider must be used within a ModelAndProviderProvider');
  }
  return context;
};
