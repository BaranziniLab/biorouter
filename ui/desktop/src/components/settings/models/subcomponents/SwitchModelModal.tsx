import { useEffect, useState, useCallback, useMemo, useRef } from 'react';
import { Brain, ExternalLink } from '../../../icons/app-icons';

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../../ui/dialog';
import { Button } from '../../../ui/button';
import { QUICKSTART_GUIDE_URL } from '../../providers/modal/constants';
import { Input } from '../../../ui/input';
import { Select } from '../../../ui/Select';
import { useConfig } from '../../../ConfigContext';
import { useModelAndProvider } from '../../../ModelAndProviderContext';
import type { View } from '../../../../utils/navigationUtils';
import Model, { getProviderMetadata, fetchModelsForProviders } from '../modelInterface';
import { getPredefinedModelsFromEnv, shouldShowPredefinedModels } from '../predefinedModelsUtils';
import { AffiliationBadge } from '../../../ui/AffiliationBadge';
import {
  affiliationPresentation,
  readProviderAffiliation,
} from '../../../privacy/providerAffiliation';
import { HostManagedModelNote } from '../../../privacy/HostManagedModelNote';
import { HOST_MANAGED_MODEL_TITLE } from '../../../privacy/hostManagedModelCopy';
import { isBrowserSurface } from '../../../../utils/surface';
import {
  llamacppStatus,
  ProviderType,
  type LlamaCppModel,
  type ProviderDetails,
  type SessionClassification,
} from '../../../../api';

// Return the first concrete model from the provider's list. The list is
// authored in priority order in the Rust provider definition (typically newest
// / preferred model first), so picking [0] gives the right default per provider
// without falling back to cross-provider name heuristics.
type ModelOption = {
  value: string;
  label: string;
  provider: string;
  providerType?: ProviderType;
  isDisabled?: boolean;
  detail?: string;
};

function findFirstAvailableModel(models: ModelOption[]): string | null {
  const validModels = models.filter(
    (m) => m.value !== 'custom' && m.value !== '__loading__' && !m.value.startsWith('__')
  );
  if (validModels.length === 0) return null;
  return validModels[0].value;
}

const llamaDownloadLabel = (model: LlamaCppModel | undefined) => {
  switch (model?.download_status) {
    case 'downloaded':
      return model.download_source === 'ollama' ? 'Downloaded in Ollama' : 'Downloaded';
    case 'partial':
      return 'Partial download';
    case 'not_downloaded':
      return 'Needs download';
    default:
      return null;
  }
};

const llamaFitLabel = (model: LlamaCppModel | undefined) => {
  switch (model?.suitability_status) {
    case 'suitable':
      return 'Recommended here';
    case 'above_recommendation':
      return `Needs ${model.recommended_gpu_memory_gib} GiB GPU memory`;
    case 'unknown_resources':
      return 'VRAM unknown';
    default:
      return null;
  }
};

const llamaModelOption = (
  modelName: string,
  catalog: Map<string, LlamaCppModel>,
  selectedProvider: ProviderDetails
): ModelOption => {
  const entry = catalog.get(modelName);
  if (!entry) {
    return {
      value: modelName,
      label: modelName,
      provider: selectedProvider.name,
      providerType: selectedProvider.provider_type,
    };
  }

  // THREE facts, not six. This row is ~46 characters wide before it wraps, and
  // the six-fact version ran to ~110 — two wrapped lines of near-identical grey
  // text per row, which is what made the list unreadable and (with the old fixed
  // row height) unclickable. What survives is what a user chooses BY: how big the
  // download is, whether they already have it, and whether it suits this machine.
  // The fallback state and the raw `owner/repo:QUANT` spec are diagnostics, not
  // selection criteria — LocalModelInventory in Settings is where those belong.
  const detail = [entry.download_size, llamaDownloadLabel(entry), llamaFitLabel(entry)]
    .filter(Boolean)
    .join(' · ');

  return {
    value: modelName,
    label: entry.display_name,
    provider: selectedProvider.name,
    providerType: selectedProvider.provider_type,
    detail,
  };
};

const modelOptionSearchText = (option: ModelOption) =>
  [option.value, option.label, option.detail].filter(Boolean).join(' ').toLowerCase();

/**
 * §14.2's pre-flight reason, rendered ON the row rather than after the attempt.
 *
 * States the rule and stops there, deliberately, because for THIS chat there is
 * no way forward to name. A row's classification only ever rises:
 * `update_session_metadata` writes `privacy_tier = CASE WHEN privacy_tier <>
 * 'public' THEN privacy_tier ELSE ?`, so a chat that has gone private cannot be
 * returned to public, and §14.6's declassification control does not exist yet.
 * Offering "make the chat public" here would name an action the user cannot
 * take. The repair that DOES exist — switch to a private model — is the set of
 * rows left enabled right beside this one, and Gate B's card is where §14.4
 * puts the buttons.
 */
const PUBLIC_MODEL_IN_PRIVATE_CHAT =
  'Unavailable: this is a private chat, so only private models may run in it';

/**
 * A provider row in the picker. `unavailableReason` is set for a provider the
 * user HAS set up that cannot run right now — the daemon's own sentence, from
 * `ProviderDetails.unavailable_reason`.
 */
type ProviderOption = { value: string; label: string; unavailableReason?: string };

/**
 * Serves a provider row as well as a model row: both carry a `label`, and only a
 * model row has a `detail` of its own, so a provider row shows its label alone
 * until there is a reason to print beneath it.
 */
const renderModelOptionLabel = (
  rawOption: unknown,
  meta: { context: 'menu' | 'value' },
  blockedReason?: string | null
) => {
  const option = rawOption as ModelOption;
  const detail = meta.context === 'menu' && blockedReason ? blockedReason : option.detail;

  if (!detail) {
    return option.label;
  }

  if (meta.context === 'value') {
    return <span className="block max-w-full truncate">{option.label}</span>;
  }

  return (
    <div className="min-w-0 py-0.5">
      <div className="truncate text-sm font-medium text-current">{option.label}</div>
      <div className="mt-0.5 line-clamp-2 text-xs leading-relaxed text-current opacity-70">
        {detail}
      </div>
    </div>
  );
};

type SwitchModelModalProps = {
  sessionId: string | null;
  onClose: () => void;
  setView: (view: View) => void;
  onModelSelected?: (model: string) => void;
  initialProvider?: string | null;
  initialModel?: string | null;
  titleOverride?: string;
  /**
   * The tier of the chat being switched (issue #56, §14.2) — "pre-flight, not
   * post-refusal". A public model in a private chat is rendered disabled with
   * the reason inline, instead of being offered, accepted, and then refused by
   * Gate A with a 409.
   *
   * `undefined` judges nothing: the settings grid opens this modal with no
   * session at all (`sessionId={null}`), and a modal that greyed out every
   * public model there would be wrong on every machine.
   */
  privacyTier?: SessionClassification;
};
export const SwitchModelModal = ({
  sessionId,
  onClose,
  setView,
  onModelSelected,
  initialProvider,
  initialModel,
  titleOverride,
  privacyTier,
}: SwitchModelModalProps) => {
  const { getProviders, getProviderModels, read } = useConfig();
  const { changeModel, currentModel, currentProvider } = useModelAndProvider();
  const [providerOptions, setProviderOptions] = useState<ProviderOption[]>([]);
  const [activeProviders, setActiveProviders] = useState<ProviderDetails[]>([]);
  const [modelOptionsByProvider, setModelOptionsByProvider] = useState<
    Record<string, ModelOption[]>
  >({});
  const [provider, setProvider] = useState<string | null>(
    initialProvider || currentProvider || null
  );
  // Only carry over the currently-running model when the provider being
  // configured matches the current chat provider. When switching providers
  // (e.g. opening the dialog with initialProvider=openai while running
  // anthropic), start empty so the auto-select effect picks the first model
  // for the selected provider instead of showing a model from the wrong one.
  const initialProviderResolved = initialProvider || currentProvider || null;
  const carryOverCurrentModel =
    !!currentModel && !!currentProvider && currentProvider === initialProviderResolved;
  const [model, setModel] = useState<string>(
    initialModel || (carryOverCurrentModel ? currentModel : '')
  );
  const [isCustomModel, setIsCustomModel] = useState(false);
  const [validationErrors, setValidationErrors] = useState({
    provider: '',
    model: '',
  });
  const [isValid, setIsValid] = useState(true);
  const [attemptedSubmit, setAttemptedSubmit] = useState(false);
  const [usePredefinedModels] = useState(shouldShowPredefinedModels());
  const [selectedPredefinedModel, setSelectedPredefinedModel] = useState<Model | null>(null);
  const [predefinedModels, setPredefinedModels] = useState<Model[]>([]);
  const [loadingModels, setLoadingModels] = useState<boolean>(false);
  const [userClearedModel, setUserClearedModel] = useState(false);
  const [providerInputValue, setProviderInputValue] = useState('');
  const [modelInputValue, setModelInputValue] = useState('');
  const loadedProvidersRef = useRef(false);

  /**
   * The providers this chat may not be switched to (§14.2, Gate A's pre-flight).
   *
   * Keyed on the same `metadata.tier` field the daemon's own
   * `available_private_providers` filters on when it builds the repair card, so
   * this list can never disagree with what a refusal would offer — and with the
   * same POLARITY. The daemon asks `metadata.tier.is_private()` and offers
   * nothing else, so the mirror here is `!== 'private'` rather than
   * `=== 'public'`.
   *
   * That distinction is load-bearing today, not just under a future third tier.
   * `ProviderMetadata::tier` is `#[serde(default)]` over a `ProviderTier` whose
   * `Default` is deliberately `Public` ("a provider module that forgets
   * `tier()` gets less reach, never more"), which is why the generated client
   * types it optional. A provider whose metadata omits the field therefore
   * arrives here as `undefined` while the daemon has already resolved it to
   * Public — `=== 'public'` would leave that row selectable and then let Gate A
   * refuse it with a 409, which is the post-refusal failure this pre-flight
   * exists to replace.
   *
   * The one way the two tiers can still differ is a provider whose shipped
   * metadata claims Private while its bound instance resolves Public (an
   * `ollama` re-pointed by `OLLAMA_HOST`). That case stays *offered* here and is
   * refused by Gate A — a missing warning, never a false one. The reverse, a
   * metadata-Public provider that is really Private, cannot occur, so nothing
   * this greys out was ever selectable.
   */
  const publicProviderNames = useMemo(
    () =>
      new Set(
        activeProviders
          .filter((provider) => provider.metadata.tier !== 'private')
          .map((provider) => provider.name)
      ),
    [activeProviders]
  );

  /**
   * DR-26's third axis for the provider selected in this modal (issue #56) —
   * *under whose agreements?*, answered **before** the switch rather than after
   * a refusal, which is the same "pre-flight, not post-refusal" rule the tier
   * pre-flight above follows.
   *
   * ⚠ **Read off the `ProviderDetails` ROW, never `metadata`.** The daemon
   * resolves this field from a live instance (`ProviderAffiliation::of`, through
   * `providers::create`), which is what makes it safe to render as a claim; the
   * metadata beside it is the type-level tier that `publicProviderNames` above
   * documents at length as unsafe to badge.
   *
   * ⚠ **The tier is deliberately NOT badged here.** The only tier this modal has
   * is `metadata.tier`, the type-level claim, and a Private pill hung on it would
   * read Private for an `ollama` re-pointed off this machine — exactly the
   * demotion the tier exists to catch. The tier's pre-flight on this surface is
   * the greyed-out row and its inline reason; the affiliation is a resolved
   * instance value and can be stated outright.
   */
  const selectedAffiliation = useMemo(
    () => readProviderAffiliation(activeProviders.find((row) => row.name === provider)),
    [activeProviders, provider]
  );
  const selectedAffiliationWords = affiliationPresentation(selectedAffiliation);

  /**
   * SD-1, at the one place every model picker in the app ends up.
   *
   * ⚠ **This is the choke point, so it is guarded here as well as at each
   * entry.** `ModelsBottomBar`, `ModelSettingsButtons`, `ProviderGrid` and
   * `ProviderGuard` all open this dialog, and each of them now refuses to on a
   * browser surface — but an entry point that forgets is a picker that submits,
   * and `changeModel` writes `BIOROUTER_PROVIDER` through
   * `/config/set_provider`, which the daemon refuses with prose written for an
   * AI agent. The same "pre-flight, not post-refusal" rule the tier check above
   * follows.
   */
  const hostManaged = isBrowserSurface();

  const blockedReasonFor = useCallback(
    (providerName: string | undefined | null) =>
      privacyTier === 'private' && providerName && publicProviderNames.has(providerName)
        ? PUBLIC_MODEL_IN_PRIVATE_CHAT
        : null,
    [privacyTier, publicProviderNames]
  );

  /**
   * F6 of the 2026-09-10 provider QA run — the same "pre-flight, not
   * post-refusal" rule as {@link blockedReasonFor}, one level up: a whole
   * PROVIDER that cannot run.
   *
   * With `CODEX_COMMAND` pointed at a path that did not exist, Codex stayed
   * selectable here, and the bind was then refused by `from_env` with a toast in
   * the far corner. The daemon now serves such a row with `is_configured: false`
   * and the reason, and the row stays in the list — disabled, with the reason on
   * it — rather than vanishing, because a provider the user chose and cannot
   * find is exactly the one they need to be told about. The words are the
   * daemon's (`CodingAgentKind::not_installed_summary`), the same sentence a turn
   * would have failed with.
   */
  const unavailableReasonFor = useCallback(
    (providerName: string | undefined | null) => {
      const reason = providerOptions.find(
        (option) => option.value === providerName
      )?.unavailableReason;
      return reason ? `Unavailable: ${reason}` : null;
    },
    [providerOptions]
  );
  const selectedProviderUnavailable = !usePredefinedModels ? unavailableReasonFor(provider) : null;

  // Validate form data
  const validateForm = useCallback(() => {
    const errors = {
      provider: '',
      model: '',
    };
    let formIsValid = true;

    if (usePredefinedModels) {
      if (!selectedPredefinedModel) {
        errors.model = 'Select a model';
        formIsValid = false;
      } else {
        // This branch swaps both selects for a flat radio list and reaches the
        // same `changeModel`, so it bypasses the option list's pre-flight
        // exactly the way the custom-model field below does. Guarding only that
        // one would leave the identical hole open on the sibling path.
        const blocked =
          unavailableReasonFor(selectedPredefinedModel.provider) ??
          blockedReasonFor(selectedPredefinedModel.provider);
        if (blocked) {
          errors.model = blocked;
          formIsValid = false;
        }
      }
    } else {
      if (!provider) {
        errors.provider = 'Select a provider';
        formIsValid = false;
      } else {
        // A disabled row cannot be picked, but the dialog can OPEN on one — the
        // bound provider, or the one a configure form just saved — and a
        // keyboard submit reaches here without the button.
        const unavailable = unavailableReasonFor(provider);
        if (unavailable) {
          errors.provider = unavailable;
          formIsValid = false;
        }
      }

      if (!model) {
        errors.model = 'Select or type a model name';
        formIsValid = false;
      }

      // The custom-model field bypasses the option list entirely, so the same
      // rule has to be asked again here or "Enter a model not listed…" would be
      // the one way around the pre-flight.
      const blocked = blockedReasonFor(provider);
      if (blocked && model) {
        errors.model = blocked;
        formIsValid = false;
      }
    }

    setValidationErrors(errors);
    setIsValid(formIsValid);
    return formIsValid;
  }, [
    model,
    provider,
    usePredefinedModels,
    selectedPredefinedModel,
    blockedReasonFor,
    unavailableReasonFor,
  ]);

  const handleClose = () => {
    onClose();
  };

  /**
   * The switch, as a state the dialog can render.
   *
   * ⚠ **A bind is not instant and can fail, and until this existed the dialog
   * admitted neither.** `changeModel` awaits two round trips to the daemon
   * (`/agent/update_provider`, then `/config/set_provider`); the button stayed
   * fully live throughout, and on failure `handleSubmit` returned without
   * closing the dialog and without writing anything into it — the sole report
   * was a toast in the far corner of the screen, which is off-screen for anyone
   * looking at the dialog they just clicked in. The bug this was reported as is
   * quotable: *"the Select Model button actually froze without telling me
   * why"*. It had not frozen. It had no way to speak.
   */
  const [switching, setSwitching] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  const handleSubmit = async () => {
    // The button below is already disabled on a browser surface; this is the
    // second half of the same guard, for a keyboard submit or a call site that
    // renders its own confirm.
    if (hostManaged) return;
    // Re-entrancy: the button is disabled while a switch is in flight, but a
    // keyboard submit reaches here directly. Two binds racing would write two
    // different providers through `/config/set_provider` and leave whichever
    // landed second as the global default.
    if (switching) return;
    if (providerInputValue.trim()) {
      setSubmitError('Choose a provider from the filtered results before continuing.');
      return;
    }
    setAttemptedSubmit(true);
    setSubmitError(null);
    const isFormValid = validateForm();
    if (!isFormValid) return;

    setSwitching(true);
    try {
      let modelObj: Model;

      if (usePredefinedModels && selectedPredefinedModel) {
        modelObj = selectedPredefinedModel;
      } else {
        const providerMetaData = await getProviderMetadata(provider || '', getProviders);
        const providerDisplayName = providerMetaData.display_name;
        modelObj = { name: model, provider: provider, subtext: providerDisplayName } as Model;
      }

      const changed = await changeModel(sessionId, modelObj);
      if (!changed) {
        // `changeModel` has already raised the toast that explains *why* — a
        // privacy barrier, a missing user proof, a provider failure. This says
        // the far plainer thing the toast cannot, in the place the user is
        // looking: the dialog is still open because nothing was switched.
        setSubmitError(
          'The model was not switched. See the notification for what went wrong, then try again.'
        );
        return;
      }

      if (onModelSelected) {
        onModelSelected(modelObj.name);
      }
      onClose();
    } catch (error) {
      // ⚠ This arm is why the dialog could hang silently. `handleSubmit` is
      // wired straight to `onClick`, so nothing holds the promise it returns: a
      // throw anywhere above became an *unhandled rejection*, the dialog
      // stayed open, and not one pixel changed. Anything that can throw here —
      // a provider-metadata fetch, an unreachable daemon — now lands in the
      // dialog instead of in the console.
      setSubmitError(
        error instanceof Error ? error.message : `The model could not be switched: ${String(error)}`
      );
    } finally {
      setSwitching(false);
    }
  };

  // Re-validate when inputs change and after attempted submission
  useEffect(() => {
    if (attemptedSubmit) {
      validateForm();
    }
  }, [attemptedSubmit, validateForm]);

  useEffect(() => {
    // Load predefined models if enabled
    if (usePredefinedModels) {
      const models = getPredefinedModelsFromEnv();
      setPredefinedModels(models);

      // Initialize selected predefined model with current model
      (async () => {
        try {
          const currentModelName =
            initialModel || ((await read('BIOROUTER_MODEL', false)) as string);
          const matchingModel = models.find((model) => model.name === currentModelName);
          if (matchingModel) {
            setSelectedPredefinedModel(matchingModel);
          }
        } catch (error) {
          console.error('Failed to get current model for selection:', error);
        }
      })();
    }

    // Load providers for manual model selection.
    //
    // getProviders(false) reuses the cached list when ConfigContext already
    // has it; getProviders(true) forces a refetch and then calls
    // setProvidersList, which mutates the ConfigContext state that the
    // useCallback closes over — that bumps the getProviders reference, which
    // re-fires THIS effect (getProviders is a dep), which calls
    // getProviders(true) again. The result is a render loop that re-fires
    // setModelOptions / setLoadingModels several times per second and makes
    // react-select flicker its ClearIndicator (the X) as it churns. The
    // cached list is fine here — the user can reopen the modal to pick up
    // newly-configured providers.
    (async () => {
      if (loadedProvidersRef.current) return;
      loadedProvidersRef.current = true;
      try {
        // Onboarding has just written a new API key, so the provider catalog
        // loaded at app startup is stale. Refresh it once before resolving the
        // detected provider; other model switches can keep using the cache.
        const providersResponse = await getProviders(Boolean(initialProvider));
        const activeProviders = providersResponse.filter((provider) => provider.is_configured);
        setActiveProviders(activeProviders);
        // Every usable provider, plus every provider the user set up that cannot
        // run right now (see `unavailableReasonFor`) — in the daemon's order, so
        // a disabled row sits where it always sat instead of sinking to the
        // bottom — then "Use other provider". A provider that is simply not set
        // up stays out, as before.
        setProviderOptions([
          ...providersResponse
            .filter((provider) => provider.is_configured || provider.unavailable_reason)
            .map(({ metadata, name, is_configured, unavailable_reason }) => ({
              value: name,
              label: metadata.display_name,
              unavailableReason: is_configured ? undefined : (unavailable_reason ?? undefined),
            })),
          {
            value: 'configure_providers',
            label: 'Use other provider',
          },
        ]);
      } catch (error: unknown) {
        console.error('Failed to query providers:', error);
      }
    })();
  }, [getProviders, initialModel, initialProvider, usePredefinedModels, read]);

  useEffect(() => {
    if (usePredefinedModels || !provider || modelOptionsByProvider[provider]) return;

    const selectedProvider = activeProviders.find((p) => p.name === provider);
    if (!selectedProvider) return;

    let cancelled = false;
    setLoadingModels(true);

    (async () => {
      try {
        const [result] = await fetchModelsForProviders([selectedProvider], getProviderModels);
        if (cancelled || !result) return;

        const modelList = result.error
          ? selectedProvider.metadata.known_models?.map(({ name }) => name) || []
          : result.models || [];

        if (result.error) {
          console.error('Provider model fetch errors:', [result.error]);
        }

        let llamaCatalog = new Map<string, LlamaCppModel>();
        if (selectedProvider.name === 'llamacpp') {
          try {
            const status = await llamacppStatus({ throwOnError: true });
            llamaCatalog = new Map(
              (status.data?.catalog || []).map((entry) => [entry.name, entry])
            );
          } catch (error) {
            console.error('Failed to query Llama Server catalog:', error);
          }
        }

        const options: ModelOption[] = modelList.map((m) =>
          selectedProvider.name === 'llamacpp'
            ? llamaModelOption(m, llamaCatalog, selectedProvider)
            : {
                value: m,
                label: m,
                provider: selectedProvider.name,
                providerType: selectedProvider.provider_type,
              }
        );

        if (selectedProvider.metadata.allows_unlisted_models) {
          options.push({
            value: 'custom',
            label: 'Enter a model not listed...',
            provider: selectedProvider.name,
            providerType: selectedProvider.provider_type,
          });
        }

        setModelOptionsByProvider((current) => ({
          ...current,
          [selectedProvider.name]: options,
        }));
      } catch (error) {
        console.error('Failed to query provider models:', error);
      } finally {
        if (!cancelled) setLoadingModels(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [activeProviders, getProviderModels, modelOptionsByProvider, provider, usePredefinedModels]);

  // Memoize so passing the same selection through to react-select doesn't
  // create a new array reference on every render — fresh references make
  // react-select treat its options list as "changed" and re-render the
  // ClearIndicator (the X button), which the user sees as a flicker while
  // interacting with the model dropdown.
  const filteredModelOptions = useMemo(() => {
    if (!provider) return [];

    const providerOptions = modelOptionsByProvider[provider] ?? [];
    const providerGroups = providerOptions.length > 0 ? [{ options: providerOptions }] : [];
    const trimmedInput = modelInputValue.trim();
    if (!trimmedInput) return providerGroups;

    const loweredInput = trimmedInput.toLowerCase();
    const matchingOptions = providerGroups
      .map((group) => ({
        options: group.options.filter(
          (option) =>
            modelOptionSearchText(option).includes(loweredInput) && option.value !== 'custom'
        ),
      }))
      .filter((group) => group.options.length > 0);

    if (matchingOptions.length > 0) return matchingOptions;

    const allowsCustomModel = providerGroups.some((group) =>
      group.options.some((option) => option.value === 'custom')
    );
    if (!allowsCustomModel) return [];

    return [
      {
        options: [
          {
            value: trimmedInput,
            label: `Use: "${trimmedInput}"`,
            provider,
          },
        ],
      },
    ];
  }, [provider, modelOptionsByProvider, modelInputValue]);

  // Same reason — a stable value object keeps the ClearIndicator stable.
  const modelSelectValue = useMemo(() => {
    if (!model) return null;

    const selectedOption = provider
      ? modelOptionsByProvider[provider]?.find((option) => option.value === model)
      : null;
    if (selectedOption) return selectedOption;
    return { value: model, label: model, provider: provider || '' };
  }, [model, modelOptionsByProvider, provider]);
  const providerSelectValue = useMemo(
    () => providerOptions.find((option) => option.value === provider) || null,
    [providerOptions, provider]
  );

  useEffect(() => {
    // Don't auto-select if user explicitly cleared the model
    if (!provider || loadingModels || model || isCustomModel || userClearedModel) return;

    const providerModels = modelOptionsByProvider[provider] ?? [];

    if (providerModels.length > 0) {
      const firstModel = findFirstAvailableModel(providerModels);
      if (firstModel) {
        setModel(firstModel);
      }
    }
  }, [provider, modelOptionsByProvider, loadingModels, model, isCustomModel, userClearedModel]);

  // Handle model selection change
  const handleModelChange = (newValue: unknown) => {
    const selectedOption = newValue as { value: string; label: string; provider: string } | null;
    if (selectedOption?.value === 'custom') {
      setIsCustomModel(true);
      setModel('');
      setProvider(selectedOption.provider);
      setUserClearedModel(false);
      setModelInputValue('');
    } else if (selectedOption === null) {
      // User cleared the selection
      setIsCustomModel(false);
      setModel('');
      setUserClearedModel(true);
      setModelInputValue('');
    } else {
      setIsCustomModel(false);
      setModel(selectedOption?.value || '');
      setProvider(selectedOption?.provider || '');
      setUserClearedModel(false);
      setModelInputValue('');
    }
  };

  const handleInputChange = (inputValue: string, actionMeta?: { action?: string }): string => {
    if (!provider) return inputValue;

    if (!actionMeta || actionMeta.action === 'input-change') {
      setModelInputValue(inputValue);
    } else if (actionMeta.action === 'menu-close') {
      setModelInputValue('');
    }

    return inputValue;
  };

  return (
    <Dialog open={true} onOpenChange={handleClose}>
      <DialogContent className="sm:max-w-[500px]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Brain size={24} className="text-text-default" />
            {titleOverride || 'Switch models'}
          </DialogTitle>
          <DialogDescription>
            {hostManaged
              ? HOST_MANAGED_MODEL_TITLE
              : 'Select a provider and model to use for your chats.'}
          </DialogDescription>
        </DialogHeader>

        {/*
          Above the controls, not below them: the reason a picker is inert has
          to be readable before the user tries it, which is the whole of SD-1's
          consequence clause. Renders nothing on the desktop.
        */}
        <HostManagedModelNote />

        <div className="flex flex-col gap-4 py-4">
          {usePredefinedModels ? (
            <div className="w-full flex flex-col gap-4">
              <div className="flex justify-between items-center">
                <label className="text-sm font-medium text-text-default">Choose a model:</label>
              </div>

              <div className="flex flex-col max-h-72 overflow-y-auto -mx-1 px-1">
                {predefinedModels.map((model) => {
                  const isSelected = selectedPredefinedModel?.name === model.name;
                  return (
                    <div
                      key={model.id || model.name}
                      // The predefined branch swaps both selects for this flat
                      // list, so it bypasses `isDisabled` on them entirely — the
                      // same hole `validateForm` documents for the tier
                      // pre-flight, and it has to be closed here for the same
                      // reason.
                      onClick={hostManaged ? undefined : () => setSelectedPredefinedModel(model)}
                      aria-disabled={hostManaged || undefined}
                      className={[
                        'biorouter-modal-row flex items-start gap-3 py-2.5 px-3 rounded-container transition-colors',
                        hostManaged ? 'cursor-not-allowed opacity-60' : 'cursor-pointer',
                        isSelected
                          ? '!border-border-default tint-selected tint-interactive'
                          : hostManaged
                            ? ''
                            : 'hover:!border-border-default tint-interactive',
                      ].join(' ')}
                    >
                      {/* Radio dot */}
                      <div
                        className={[
                          'mt-0.5 h-4 w-4 rounded-full border-2 flex-shrink-0 flex items-center justify-center transition-all duration-150',
                          isSelected
                            ? 'border-text-default bg-text-default'
                            : 'border-border-subtle',
                        ].join(' ')}
                      >
                        {isSelected && (
                          <div className="h-1.5 w-1.5 rounded-full bg-background-default" />
                        )}
                      </div>

                      {/* Model info */}
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2 flex-wrap">
                          <span className="text-sm font-medium text-text-default">
                            {model.alias || model.name}
                          </span>
                          {model.alias?.toLowerCase().includes('recommended') && (
                            <span className="text-caps text-text-muted bg-background-default/80 px-1.5 py-0.5 rounded-element shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--border-subtle)_55%,transparent)]">
                              Recommended
                            </span>
                          )}
                        </div>
                        {model.subtext && (
                          <p className="text-xs text-text-muted mt-0.5 line-clamp-2 leading-relaxed">
                            {model.subtext}
                          </p>
                        )}
                      </div>

                      {/* Provider badge */}
                      {model.provider && (
                        <span className="text-supporting text-text-muted bg-background-default/80 px-1.5 py-0.5 rounded-element flex-shrink-0 mt-0.5 shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--border-subtle)_55%,transparent)]">
                          {model.provider}
                        </span>
                      )}
                    </div>
                  );
                })}
              </div>

              {attemptedSubmit && validationErrors.model && (
                <div className="text-text-danger text-sm mt-1">{validationErrors.model}</div>
              )}
            </div>
          ) : (
            /* Manual Provider/Model Selection */
            <div className="w-full flex flex-col gap-4">
              <div>
                <Select
                  options={providerOptions}
                  value={providerSelectValue}
                  inputValue={providerInputValue}
                  onInputChange={(inputValue: string, actionMeta?: { action?: string }) => {
                    if (!actionMeta || actionMeta.action === 'input-change') {
                      setProviderInputValue(inputValue);
                    } else if (actionMeta.action === 'menu-close') {
                      setProviderInputValue('');
                    }
                    return inputValue;
                  }}
                  onChange={(newValue: unknown) => {
                    const option = newValue as { value: string; label: string } | null;
                    setProviderInputValue('');
                    if (option?.value === 'configure_providers') {
                      // Navigate to ConfigureProviders view
                      setView('ConfigureProviders');
                      onClose(); // Close the current modal
                    } else {
                      setProvider(option?.value || null);
                      setModel('');
                      setIsCustomModel(false);
                      setUserClearedModel(false);
                      setModelInputValue('');
                    }
                  }}
                  placeholder="Provider, type to search"
                  isClearable
                  isDisabled={hostManaged}
                  // The private-chat pre-flight's shape, one level up: the row is
                  // react-select's own `aria-disabled` option, with the reason in
                  // its detail line — see `unavailableReasonFor`.
                  formatOptionLabel={(rawOption: unknown, meta) =>
                    renderModelOptionLabel(
                      rawOption,
                      meta,
                      unavailableReasonFor((rawOption as ProviderOption).value)
                    )
                  }
                  isOptionDisabled={(rawOption: unknown) =>
                    unavailableReasonFor((rawOption as ProviderOption).value) !== null
                  }
                />
                {/* Shown before any attempt when the dialog opened ON an
                    unavailable provider: the reason the button below is inert
                    has to be readable before the user tries it. */}
                {(selectedProviderUnavailable ||
                  (attemptedSubmit && validationErrors.provider)) && (
                  <div
                    data-testid="switch-model-provider-error"
                    className="text-text-danger text-sm mt-1"
                  >
                    {selectedProviderUnavailable || validationErrors.provider}
                  </div>
                )}
                {/*
                  Issue #56, DR-26. Whose agreements cover the models under this
                  provider, stated before the user picks one. Nothing renders for
                  a public provider — it has no affiliation — so this row appears
                  exactly when there is something to say.
                */}
                {selectedAffiliationWords && (
                  <div
                    data-testid="switch-model-affiliation"
                    className="mt-2 flex items-start gap-2"
                  >
                    <AffiliationBadge affiliation={selectedAffiliation} className="mt-0.5" />
                    <p className="min-w-0 flex-1 text-[11px] leading-4 text-text-muted [overflow-wrap:anywhere]">
                      {selectedAffiliationWords.title}
                    </p>
                  </div>
                )}
              </div>

              {provider && (
                <>
                  {!isCustomModel ? (
                    <div>
                      <Select
                        options={loadingModels ? [] : filteredModelOptions}
                        onChange={handleModelChange}
                        onInputChange={handleInputChange}
                        inputValue={modelInputValue}
                        value={modelSelectValue}
                        formatOptionLabel={(rawOption: unknown, meta) =>
                          renderModelOptionLabel(
                            rawOption,
                            meta,
                            blockedReasonFor((rawOption as ModelOption).provider)
                          )
                        }
                        isOptionDisabled={(rawOption: unknown) =>
                          blockedReasonFor((rawOption as ModelOption).provider) !== null
                        }
                        placeholder={
                          loadingModels ? 'Loading models…' : 'Select a model, type to search'
                        }
                        isClearable
                        isDisabled={
                          loadingModels || hostManaged || selectedProviderUnavailable !== null
                        }
                      />

                      {attemptedSubmit && validationErrors.model && (
                        <div className="text-text-danger text-sm mt-1">
                          {validationErrors.model}
                        </div>
                      )}
                    </div>
                  ) : (
                    <div className="flex flex-col gap-2">
                      <div className="flex justify-between">
                        <label className="text-sm text-text-muted">Custom model name</label>
                        <button
                          onClick={() => setIsCustomModel(false)}
                          className="text-sm text-text-muted"
                        >
                          Back to model list
                        </button>
                      </div>
                      <Input
                        className="border-2 px-4 py-5"
                        placeholder="Type model name here"
                        onChange={(event) => setModel(event.target.value)}
                        value={model}
                        disabled={hostManaged}
                      />
                      {attemptedSubmit && validationErrors.model && (
                        <div className="text-text-danger text-sm mt-1">
                          {validationErrors.model}
                        </div>
                      )}
                    </div>
                  )}
                </>
              )}
            </div>
          )}
        </div>

        {submitError && (
          <div
            data-testid="switch-model-submit-error"
            role="alert"
            className="rounded-container border border-border-subtle bg-background-muted px-3 py-2.5 text-sm leading-relaxed text-text-danger [overflow-wrap:anywhere]"
          >
            {submitError}
          </div>
        )}

        <DialogFooter className="pt-4 flex-col sm:flex-row gap-3">
          <a
            href={QUICKSTART_GUIDE_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center text-text-muted hover:text-text-default text-sm mr-auto"
          >
            <ExternalLink size={14} className="mr-1" />
            Quick start guide
          </a>
          <div className="flex gap-2">
            <Button variant="outline" onClick={handleClose} type="button">
              Cancel
            </Button>
            <Button
              onClick={handleSubmit}
              disabled={
                !isValid ||
                (!usePredefinedModels && (!provider || !model)) ||
                selectedProviderUnavailable !== null ||
                hostManaged ||
                switching ||
                providerInputValue.trim().length > 0
              }
            >
              {switching ? 'Switching\u2026' : 'Select model'}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
