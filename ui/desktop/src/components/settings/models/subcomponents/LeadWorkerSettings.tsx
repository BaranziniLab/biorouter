import { useState, useEffect } from 'react';
import { useConfig } from '../../../ConfigContext';
import { useModelAndProvider } from '../../../ModelAndProviderContext';
import { Button } from '../../../ui/button';
import { Select } from '../../../ui/Select';
import { Input } from '../../../ui/input';
import { Switch } from '../../../ui/switch';
import { getPredefinedModelsFromEnv, shouldShowPredefinedModels } from '../predefinedModelsUtils';
import { fetchModelsForProviders } from '../modelInterface';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '../../../ui/dialog';
import { HostManagedModelNote } from '../../../privacy/HostManagedModelNote';
import { HOST_MANAGED_MODEL_REASON } from '../../../privacy/hostManagedModelCopy';
import { isBrowserSurface } from '../../../../utils/surface';

interface LeadWorkerSettingsProps {
  isOpen: boolean;
  onClose: () => void;
}

/**
 * A model row in either select.
 *
 * `unavailableReason` is the daemon's own sentence for a provider the user HAS
 * set up that cannot run right now (`ProviderDetails.unavailable_reason`, from
 * `ProviderReadiness::Unavailable`).
 *
 * # The defect (D4 of the 2026-09-12 model-controls run)
 *
 * This dialog built its options from `providers.filter((p) => p.is_configured)`
 * and mapped them to `{ value, label, provider }`, dropping the row. The daemon
 * serves an unavailable provider `is_configured: false` **with** a reason, so the
 * filter deleted the one row the user most needs to see — and the option shape it
 * mapped to had nowhere to carry the reason even if the row had survived.
 *
 * Measured live (dev GUI, sandboxed config, `CODEX_COMMAND: /nope/codex`,
 * 2026-09-12): this dialog built **25** options, `anyCodex=false`,
 * `anyUnavailable=false`, while `GET /config/providers` was serving
 * `codex | is_configured=False | unavailable_reason='Codex is not installed, or is
 * not on a path Biorouter searches' | 6 known models`. One menu item away, the
 * Switch-models picker rendered that provider disabled with the same sentence on
 * the row.
 *
 * So the reason is carried through, and rendered the way that picker already
 * renders it: `aria-disabled` rows with the sentence beneath the label, plus the
 * sentence beside a FIELD whose selection is barred — because a disabled row is
 * not a disabled selection (a pair saved while the CLI worked reopens here after
 * it moved, with nobody re-picking anything).
 */
type LeadWorkerModelOption = {
  value: string;
  label: string;
  provider: string;
  unavailableReason?: string;
};

/** The prefix the Switch-models picker uses, so the two surfaces read alike. */
const unavailableLine = (reason: string) => `Unavailable: ${reason}`;

/**
 * A row's label, with the reason on a second line in the MENU only — the closed
 * field has room for the name alone, and the reason is stated beside it instead.
 */
const renderLeadWorkerOption = (rawOption: unknown, meta: { context: 'menu' | 'value' }) => {
  const option = rawOption as LeadWorkerModelOption;
  if (meta.context === 'value' || !option.unavailableReason) {
    return <span className="block max-w-full truncate">{option.label}</span>;
  }
  return (
    <div className="min-w-0 py-0.5">
      <div className="truncate text-sm font-medium text-current">{option.label}</div>
      <div className="mt-0.5 line-clamp-2 text-xs leading-relaxed text-current opacity-70">
        {unavailableLine(option.unavailableReason)}
      </div>
    </div>
  );
};

export function LeadWorkerSettings({ isOpen, onClose }: LeadWorkerSettingsProps) {
  const { read, upsert, getProviders, getProviderModels, remove } = useConfig();
  const { currentModel } = useModelAndProvider();
  const [leadModel, setLeadModel] = useState<string>('');
  const [workerModel, setWorkerModel] = useState<string>('');
  const [leadProvider, setLeadProvider] = useState<string>('');
  const [workerProvider, setWorkerProvider] = useState<string>('');
  // Minimal custom model mode toggles
  const [isLeadCustomModel, setIsLeadCustomModel] = useState<boolean>(false);
  const [isWorkerCustomModel, setIsWorkerCustomModel] = useState<boolean>(false);
  const [leadTurns, setLeadTurns] = useState<number>(3);
  const [failureThreshold, setFailureThreshold] = useState<number>(2);
  const [fallbackTurns, setFallbackTurns] = useState<number>(2);
  const [isEnabled, setIsEnabled] = useState(false);
  const [modelOptions, setModelOptions] = useState<LeadWorkerModelOption[]>([]);
  const [isLoading, setIsLoading] = useState(true);

  // Load current configuration
  useEffect(() => {
    if (!isOpen) return; // Only load when modal is open

    const loadConfig = async () => {
      try {
        setIsLoading(true);
        const [
          leadModelConfig,
          leadProviderConfig,
          leadTurnsConfig,
          failureThresholdConfig,
          fallbackTurnsConfig,
        ] = await Promise.all([
          read('BIOROUTER_LEAD_MODEL', false),
          read('BIOROUTER_LEAD_PROVIDER', false),
          read('BIOROUTER_LEAD_TURNS', false),
          read('BIOROUTER_LEAD_FAILURE_THRESHOLD', false),
          read('BIOROUTER_LEAD_FALLBACK_TURNS', false),
        ]);

        if (leadModelConfig) {
          setLeadModel(leadModelConfig as string);
          setIsEnabled(true);
        } else {
          setLeadModel('');
          setIsEnabled(false);
        }
        if (leadProviderConfig) setLeadProvider(leadProviderConfig as string);
        else setLeadProvider('');
        if (leadTurnsConfig) setLeadTurns(Number(leadTurnsConfig));
        else setLeadTurns(3);
        if (failureThresholdConfig) setFailureThreshold(Number(failureThresholdConfig));
        else setFailureThreshold(2);
        if (fallbackTurnsConfig) setFallbackTurns(Number(fallbackTurnsConfig));
        else setFallbackTurns(2);

        // Set worker model to current model or from config
        const workerModelConfig = await read('BIOROUTER_MODEL', false);
        if (workerModelConfig) {
          setWorkerModel(workerModelConfig as string);
        } else if (currentModel) {
          setWorkerModel(currentModel as string);
        } else {
          setWorkerModel('');
        }

        const workerProviderConfig = await read('BIOROUTER_PROVIDER', false);
        if (workerProviderConfig) {
          setWorkerProvider(workerProviderConfig as string);
        } else {
          setWorkerProvider('');
        }

        // Load available models
        const options: LeadWorkerModelOption[] = [];

        if (shouldShowPredefinedModels()) {
          // Use predefined models if available
          const predefinedModels = getPredefinedModelsFromEnv();
          predefinedModels.forEach((model) => {
            options.push({
              value: model.name, // Use name for switching
              label: model.alias || model.name, // Use alias for display, fallback to name
              provider: model.provider,
            });
          });
        } else {
          // Fallback to provider-based models
          const providers = await getProviders(false);
          // D4 — every usable provider, PLUS every provider the user set up that
          // cannot run right now, which arrives `is_configured: false` with a
          // reason. The reason rides the option from here to the row; a provider
          // that was never set up stays out, exactly as before. Same predicate,
          // same polarity, as `SwitchModelModal`'s provider list.
          const listedProviders = providers.filter((p) => p.is_configured || p.unavailable_reason);

          const results = await fetchModelsForProviders(listedProviders, getProviderModels);
          results.forEach(({ provider: p, models, error }) => {
            if (error) {
              console.error(error);
            }
            const unavailableReason = p.is_configured
              ? undefined
              : (p.unavailable_reason ?? undefined);

            if (models && models.length > 0) {
              models.forEach((modelName) => {
                options.push({
                  value: modelName,
                  label: `${modelName} (${p.metadata.display_name})`,
                  provider: p.name,
                  unavailableReason,
                });
              });
            }
            // Add custom model option for all non-Custom providers
            if (p.provider_type !== 'Custom') {
              options.push({
                value: `__custom__:${p.name}`,
                label: 'Enter a model not listed...',
                provider: p.name,
                unavailableReason,
              });
            }
          });
        }

        setModelOptions(options);
      } catch (error) {
        console.error('Error loading configuration:', error);
      } finally {
        setIsLoading(false);
      }
    };

    loadConfig();
  }, [read, getProviders, getProviderModels, currentModel, isOpen]);

  // If current models are not in the list (e.g., previously set to custom), switch to custom mode
  useEffect(() => {
    if (!isLoading) {
      if (leadModel && !modelOptions.find((opt) => opt.value === leadModel)) {
        setIsLeadCustomModel(true);
      }
      if (workerModel && !modelOptions.find((opt) => opt.value === workerModel)) {
        setIsWorkerCustomModel(true);
      }
    }
  }, [isLoading, modelOptions, leadModel, workerModel]);

  /**
   * SD-1. Save writes `BIOROUTER_LEAD_MODEL`, `BIOROUTER_LEAD_PROVIDER` and
   * `BIOROUTER_PROVIDER` — three of the five capability keys — and the disable
   * branch *removes* two of them, which `/config/remove` guards identically.
   * Both directions 409 in a browser.
   */
  const hostManaged = isBrowserSurface();

  /**
   * D4's pre-flight: the reason each HALF cannot run, derived from the selection
   * on every render.
   *
   * ⚠ **A disabled row is not a disabled selection** — the rule
   * `SwitchModelModal.validation` records at length, and it bites harder here
   * because nothing in this dialog picks anything on open: the two fields are
   * filled from `BIOROUTER_LEAD_*` / `BIOROUTER_MODEL`, so a pair saved while a
   * coding agent's CLI resolved reopens after it moved with a barred model in a
   * field whose menu nobody will touch. Keyed on the PROVIDER, because that is
   * what the reason belongs to, and read off the option list so it can never
   * disagree with the row.
   */
  const unavailableReasonForProvider = (providerName: string) =>
    modelOptions.find((option) => option.provider === providerName)?.unavailableReason ?? null;
  const leadUnavailable = leadProvider ? unavailableReasonForProvider(leadProvider) : null;
  const workerUnavailable = workerProvider ? unavailableReasonForProvider(workerProvider) : null;
  // Only what is actually in force: the fields are inert while the pair is off.
  const barred = isEnabled && !!(leadUnavailable || workerUnavailable);

  const handleSave = async () => {
    if (hostManaged) return;
    // The button below is already disabled on this verdict; this is its
    // post-click half, for a submit that arrives some other way.
    if (barred) return;
    try {
      if (isEnabled && leadModel && workerModel) {
        // Save lead/worker configuration
        await Promise.all([
          upsert('BIOROUTER_LEAD_MODEL', leadModel, false),
          leadProvider && upsert('BIOROUTER_LEAD_PROVIDER', leadProvider, false),
          upsert('BIOROUTER_MODEL', workerModel, false),
          workerProvider && upsert('BIOROUTER_PROVIDER', workerProvider, false),
          upsert('BIOROUTER_LEAD_TURNS', leadTurns, false),
          upsert('BIOROUTER_LEAD_FAILURE_THRESHOLD', failureThreshold, false),
          upsert('BIOROUTER_LEAD_FALLBACK_TURNS', fallbackTurns, false),
        ]);
      } else {
        // Remove lead/worker configuration
        await Promise.all([
          remove('BIOROUTER_LEAD_MODEL', false),
          remove('BIOROUTER_LEAD_PROVIDER', false),
          remove('BIOROUTER_LEAD_TURNS', false),
          remove('BIOROUTER_LEAD_FAILURE_THRESHOLD', false),
          remove('BIOROUTER_LEAD_FALLBACK_TURNS', false),
        ]);
      }
      onClose();
    } catch (error) {
      console.error('Error saving configuration:', error);
    }
  };

  if (isLoading) {
    return (
      <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
        <DialogContent aria-describedby={undefined} className="p-0 sm:max-w-[560px]">
          <DialogHeader className="px-5 pb-2 pt-5">
            <DialogTitle>Lead/worker mode</DialogTitle>
          </DialogHeader>
          <div className="px-5 pb-5 text-sm text-text-muted">Loading...</div>
        </DialogContent>
      </Dialog>
    );
  }

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-h-[min(720px,calc(100vh-2rem))] overflow-y-auto p-0 sm:max-w-[620px]">
        <DialogHeader className="px-5 pb-2 pt-5">
          <DialogTitle>Lead/worker mode</DialogTitle>
        </DialogHeader>
        <div className="space-y-5 px-5 pb-5">
          <DialogDescription className="text-sm text-text-muted">
            Configure a lead model for planning and a worker model for execution.
          </DialogDescription>

          <HostManagedModelNote />

          <div className="biorouter-modal-panel flex items-center justify-between gap-4 rounded-container px-3 py-2.5">
            <div>
              <label htmlFor="enable-lead-worker" className="text-sm font-medium text-text-default">
                Lead/worker mode
              </label>
              <p className="mt-0.5 text-xs text-text-muted">
                Route planning to a lead model and routine work to a worker model.
              </p>
            </div>
            <Switch
              id="enable-lead-worker"
              checked={isEnabled}
              onCheckedChange={setIsEnabled}
              variant="mono"
            />
          </div>

          <div className={`space-y-4 ${!isEnabled ? 'opacity-60' : ''}`}>
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <label className="text-sm font-medium text-text-default">Lead model</label>
                {isLeadCustomModel && (
                  <button
                    onClick={() => setIsLeadCustomModel(false)}
                    className="rounded px-1.5 py-1 text-xs text-text-muted transition-colors hover:bg-background-muted hover:text-text-default"
                    type="button"
                  >
                    Back to model list
                  </button>
                )}
              </div>
              {!isLeadCustomModel ? (
                <Select
                  options={modelOptions}
                  value={
                    leadModel ? modelOptions.find((opt) => opt.value === leadModel) || null : null
                  }
                  onChange={(newValue: unknown) => {
                    const option = newValue as { value: string; provider: string } | null;
                    if (option) {
                      if (option.value.startsWith('__custom__')) {
                        setIsLeadCustomModel(true);
                        setLeadModel('');
                        setLeadProvider(option.provider);
                        return;
                      }
                      setLeadModel(option.value);
                      setLeadProvider(option.provider);
                    }
                  }}
                  placeholder="Select lead model..."
                  isDisabled={!isEnabled}
                  formatOptionLabel={renderLeadWorkerOption}
                  isOptionDisabled={(rawOption: unknown) =>
                    !!(rawOption as LeadWorkerModelOption).unavailableReason
                  }
                />
              ) : (
                <Input
                  className="mb-2 h-[38px]"
                  placeholder="Type model name here"
                  onChange={(event) => setLeadModel(event.target.value)}
                  value={leadModel}
                  disabled={!isEnabled}
                />
              )}
              {/* D4 — beside the field, because a disabled row says nothing about
                  a selection nobody re-picked. The custom-model input above
                  bypasses the option list entirely and keeps its provider, so the
                  same verdict has to be spoken here for both branches. */}
              {isEnabled && leadUnavailable ? (
                <p data-testid="lead-worker-lead-unavailable" className="text-sm text-text-danger">
                  {unavailableLine(leadUnavailable)}
                </p>
              ) : null}
              <p className="text-xs text-text-muted">
                Strong model for initial planning and fallback recovery
              </p>
            </div>

            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <label className="text-sm font-medium text-text-default">Worker model</label>
                {isWorkerCustomModel && (
                  <button
                    onClick={() => setIsWorkerCustomModel(false)}
                    className="rounded px-1.5 py-1 text-xs text-text-muted transition-colors hover:bg-background-muted hover:text-text-default"
                    type="button"
                  >
                    Back to model list
                  </button>
                )}
              </div>
              {!isWorkerCustomModel ? (
                <Select
                  options={modelOptions}
                  value={
                    workerModel
                      ? modelOptions.find((opt) => opt.value === workerModel) || null
                      : null
                  }
                  onChange={(newValue: unknown) => {
                    const option = newValue as { value: string; provider: string } | null;
                    if (option) {
                      if (option.value.startsWith('__custom__')) {
                        setIsWorkerCustomModel(true);
                        setWorkerModel('');
                        setWorkerProvider(option.provider);
                        return;
                      }
                      setWorkerModel(option.value);
                      setWorkerProvider(option.provider);
                    }
                  }}
                  placeholder="Select worker model..."
                  isDisabled={!isEnabled}
                  formatOptionLabel={renderLeadWorkerOption}
                  isOptionDisabled={(rawOption: unknown) =>
                    !!(rawOption as LeadWorkerModelOption).unavailableReason
                  }
                />
              ) : (
                <Input
                  className="mb-2 h-[38px]"
                  placeholder="Type model name here"
                  onChange={(event) => setWorkerModel(event.target.value)}
                  value={workerModel}
                  disabled={!isEnabled}
                />
              )}
              {isEnabled && workerUnavailable ? (
                <p
                  data-testid="lead-worker-worker-unavailable"
                  className="text-sm text-text-danger"
                >
                  {unavailableLine(workerUnavailable)}
                </p>
              ) : null}
              <p className="text-xs text-text-muted">Fast model for routine execution tasks</p>
            </div>

            <div className="biorouter-modal-panel grid grid-cols-3 gap-3 rounded-container p-3">
              <div className="space-y-2">
                <label className="flex items-center gap-1 text-sm font-medium text-text-default">
                  Initial lead turns
                </label>
                <Input
                  type="number"
                  min={1}
                  max={10}
                  value={leadTurns}
                  onChange={(e) => setLeadTurns(Number(e.target.value))}
                  className="w-full"
                  disabled={!isEnabled}
                />
                <p className="text-xs text-text-muted">Lead turns at start</p>
              </div>

              <div className="space-y-2">
                <label className="flex items-center gap-1 text-sm font-medium text-text-default">
                  Failure threshold
                </label>
                <Input
                  type="number"
                  min={1}
                  max={5}
                  value={failureThreshold}
                  onChange={(e) => setFailureThreshold(Number(e.target.value))}
                  className="w-full"
                  disabled={!isEnabled}
                />
                <p className="text-xs text-text-muted">Failures before fallback</p>
              </div>

              <div className="space-y-2">
                <label className="flex items-center gap-1 text-sm font-medium text-text-default">
                  Fallback turns
                </label>
                <Input
                  type="number"
                  min={1}
                  max={5}
                  value={fallbackTurns}
                  onChange={(e) => setFallbackTurns(Number(e.target.value))}
                  className="w-full"
                  disabled={!isEnabled}
                />
                <p className="text-xs text-text-muted">Lead turns during fallback</p>
              </div>
            </div>
          </div>

          <div className="flex justify-end gap-2 pt-1">
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button
              onClick={handleSave}
              disabled={hostManaged || barred || (isEnabled && (!leadModel || !workerModel))}
              title={hostManaged ? HOST_MANAGED_MODEL_REASON : undefined}
            >
              Save settings
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
