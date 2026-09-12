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
  const [modelOptions, setModelOptions] = useState<
    { value: string; label: string; provider: string }[]
  >([]);
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
        const options: { value: string; label: string; provider: string }[] = [];

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
          const activeProviders = providers.filter((p) => p.is_configured);

          const results = await fetchModelsForProviders(activeProviders, getProviderModels);
          results.forEach(({ provider: p, models, error }) => {
            if (error) {
              console.error(error);
            }

            if (models && models.length > 0) {
              models.forEach((modelName) => {
                options.push({
                  value: modelName,
                  label: `${modelName} (${p.metadata.display_name})`,
                  provider: p.name,
                });
              });
            }
            // Add custom model option for all non-Custom providers
            if (p.provider_type !== 'Custom') {
              options.push({
                value: `__custom__:${p.name}`,
                label: 'Enter a model not listed...',
                provider: p.name,
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

  const handleSave = async () => {
    if (hostManaged) return;
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
              disabled={hostManaged || (isEnabled && (!leadModel || !workerModel))}
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
