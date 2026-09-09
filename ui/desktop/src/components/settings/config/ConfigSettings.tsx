import { useState, useEffect, useMemo } from 'react';
import { Input } from '../../ui/input';
import { Button } from '../../ui/button';
import { useConfig } from '../../ConfigContext';
import { useModelAndProvider } from '../../ModelAndProviderContext';
import { cn } from '../../../utils';
import { Save, RotateCcw, FileText, Loader2, Settings } from '../../icons/app-icons';
import { toastSuccess, toastError } from '../../../toasts';
import { getUiNames, providerPrefixes } from '../../../utils/configUtils';
import { isBrowserSurface, isHostManagedConfigKey } from '../../../utils/surface';
import { HOST_MANAGED_MODEL_REASON } from '../../privacy/hostManagedModelCopy';
import { HostManagedModelNote } from '../../privacy/HostManagedModelNote';
import { MODAL_SIZE } from '../../ModalShell';
import type { ConfigData, ConfigValue } from '../../../types/config';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '../../ui/dialog';

export default function ConfigSettings() {
  const { config, upsert, refreshConfig } = useConfig();
  const { currentProvider: liveProvider, refreshCurrentModelAndProvider } = useModelAndProvider();
  const typedConfig = config as ConfigData;
  const [configValues, setConfigValues] = useState<ConfigData>({});
  const [modifiedKeys, setModifiedKeys] = useState<Set<string>>(new Set());
  const [saving, setSaving] = useState<string | null>(null);
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [originalKeyOrder, setOriginalKeyOrder] = useState<string[]>([]);

  useEffect(() => {
    setConfigValues(typedConfig);
    setModifiedKeys(new Set());

    // Capture the original key order only on first load or when new keys are added
    const currentKeys = Object.keys(typedConfig);
    setOriginalKeyOrder((prevOrder) => {
      if (prevOrder.length === 0) {
        // First load - capture the initial order
        return currentKeys;
      } else if (currentKeys.length > prevOrder.length) {
        // New keys have been added - add them to the end while preserving existing order
        const newKeys = currentKeys.filter((key) => !prevOrder.includes(key));
        return [...prevOrder, ...newKeys];
      }
      // Don't reorder when keys are just updated/saved - preserve the original order
      return prevOrder;
    });
  }, [typedConfig]);

  const handleChange = (key: string, value: string) => {
    setConfigValues((prev: ConfigData) => ({
      ...prev,
      [key]: value,
    }));

    setModifiedKeys((prev) => {
      const newSet = new Set(prev);
      if (value !== String(typedConfig[key] || '')) {
        newSet.add(key);
      } else {
        newSet.delete(key);
      }
      return newSet;
    });
  };

  /**
   * SD-1, key by key.
   *
   * ⚠ **Not a blanket disable.** This editor renders every non-secret config
   * key, and a browser-served daemon refuses exactly five of them — the ones
   * `is_capability_key` names. Greying out the whole page would be wrong about
   * the great majority of it, so the question is asked per row. See
   * `utils/surface.ts` for the mirrored list and the drift risk it carries.
   */
  const hostManaged = isBrowserSurface();
  const isFixedByHost = (key: string) => hostManaged && isHostManagedConfigKey(key);

  const handleSave = async (key: string) => {
    if (isFixedByHost(key)) return;
    setSaving(key);
    try {
      await upsert(key, configValues[key], false);
      toastSuccess({
        title: 'Configuration updated',
        msg: `Saved "${getUiNames(key)}"`,
      });

      // Remove this key from modified keys since it's now saved
      setModifiedKeys((prev) => {
        const newSet = new Set(prev);
        newSet.delete(key);
        return newSet;
      });
    } catch (error) {
      console.error('Failed to save config:', error);
      toastError({
        title: 'Save failed',
        msg: `Failed to save "${getUiNames(key)}"`,
        traceback: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setSaving(null);
    }
  };

  const handleReset = () => {
    setConfigValues(typedConfig);
    setModifiedKeys(new Set());
    toastSuccess({
      title: 'Configuration reset',
      msg: 'All changes have been reverted',
    });
  };

  const handleModalClose = (open: boolean) => {
    if (!open && modifiedKeys.size > 0) {
      // Reset any unsaved changes when closing the modal
      setConfigValues(typedConfig);
      setModifiedKeys(new Set());
    }
    setIsModalOpen(open);
  };

  // #50 — the ACTIVE provider, not the one this page happened to boot with.
  // ConfigContext's `config` is refetched only when a write goes through that
  // context, and switching model/provider writes straight to the API via
  // `setConfigProvider` — so the cached copy keeps naming the provider that was
  // configured at startup (the reported "current settings for ollama" while
  // versa_azure was active). ModelAndProviderContext tracks the live one; the
  // cached config value is only the fallback for the first paint, before the
  // live value has loaded.
  const currentProvider = liveProvider || typedConfig.BIOROUTER_PROVIDER || '';

  // Opening Settings re-reads the provider from the backing config, so the
  // label is also correct after a change made outside this renderer session.
  //
  // The editable rows below come from the cached config, which no model switch
  // invalidates (#52), so re-read that too — otherwise this page shows the live
  // provider in its heading and the pre-switch BIOROUTER_PROVIDER/BIOROUTER_MODEL
  // in the fields the user is about to edit.
  //
  // `refreshConfig` rejects when the read fails — that is what stops a failed
  // read from erasing the cache — so this must handle it rather than discard
  // the promise. There is nothing for this page to do about it: the snapshot it
  // renders is the one already in hand, which the failed read deliberately left
  // alone. Dropping the rejection on the floor instead would make opening
  // Settings against an unhealthy daemon an unhandled rejection.
  useEffect(() => {
    refreshCurrentModelAndProvider();
    refreshConfig().catch((error) => {
      console.error('Failed to re-read the cached config when opening Settings:', error);
    });
  }, [refreshConfig, refreshCurrentModelAndProvider]);

  const configEntries: [string, ConfigValue][] = useMemo(() => {
    const currentProviderPrefixes = providerPrefixes[currentProvider] || [];
    const allProviderPrefixes = Object.values(providerPrefixes).flat();

    return originalKeyOrder
      .filter((key) => {
        // skip secrets and internal/hidden keys
        if (
          key === 'extensions' ||
          key === 'BIOROUTER_TELEMETRY_ENABLED' ||
          key === 'tunnel_auto_start' ||
          key.includes('_KEY') ||
          key.includes('_TOKEN')
        ) {
          return false;
        }

        // Only show provider-specific entries for the current provider
        const providerSpecific = allProviderPrefixes.some((prefix: string) =>
          key.startsWith(prefix)
        );
        if (providerSpecific) {
          return currentProviderPrefixes.some((prefix: string) => key.startsWith(prefix));
        }

        return true;
      })
      .map((key) => [key, configValues[key]]);
  }, [originalKeyOrder, configValues, currentProvider]);

  return (
    <div className="biorouter-settings-section">
      <div className="biorouter-settings-section-header">
        <h2 className="text-caps text-text-muted mb-1">Configuration</h2>
        <p className="text-supporting text-text-muted">
          Edit your Biorouter configuration settings
          {currentProvider && ` (current settings for ${currentProvider})`}
        </p>
      </div>
      <div className="biorouter-settings-control-strip">
        <Dialog open={isModalOpen} onOpenChange={handleModalClose}>
          <DialogTrigger asChild>
            <Button variant="secondary">
              <Settings className="h-4 w-4" />
              Edit configuration
            </Button>
          </DialogTrigger>
          {/* `MODAL_SIZE.lg`, not `max-w-4xl` — which additionally carried no
              `sm:` prefix and so ate the primitive's small-window gutter. */}
          <DialogContent className={`${MODAL_SIZE.lg} max-h-[80vh]`}>
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2">
                {/* `text-iconStandard` is not a token: it had no definition and
                    no effect. */}
                <FileText size={20} />
                Configuration editor
              </DialogTitle>
              <DialogDescription>
                Edit your biorouter configuration settings
                {currentProvider && ` (current settings for ${currentProvider})`}
              </DialogDescription>
            </DialogHeader>

            <div className="flex-1 max-h-[60vh] overflow-auto pr-4">
              <div className="space-y-4">
                {configEntries.length === 0 ? (
                  <p className="text-text-muted">No configuration settings found.</p>
                ) : (
                  configEntries.map(([key, _value]) => {
                    const fixedByHost = isFixedByHost(key);
                    return (
                      <div
                        key={key}
                        className="grid grid-cols-[minmax(0,200px)_1fr_auto] items-center gap-3"
                      >
                        <label className="text-label text-text-default" title={key}>
                          {getUiNames(key)}
                        </label>
                        <div className="min-w-0">
                          <Input
                            value={String(configValues[key] || '')}
                            onChange={(e) => handleChange(key, e.target.value)}
                            disabled={fixedByHost}
                            // ⚠ Only the modified-key marker survives. The three
                            // deleted overrides each fought the primitive:
                            // `border-border-subtle` replaced the input's own
                            // `--border-emphasized` with the divider hairline,
                            // `hover:border-border-subtle` pinned hover to the
                            // resting colour while the primitive's inset ring
                            // still fired, and `transition-colors` REPLACED the
                            // input's transition list, dropping `box-shadow`
                            // from it.
                            className={cn(modifiedKeys.has(key) && 'border-border-info')}
                            placeholder={`Enter ${getUiNames(key)}`}
                          />
                          {/* The `fixedByHost &&` guard is load-bearing and stays:
                              it carries the per-key `isHostManagedConfigKey`
                              half, which the note itself cannot know. What went
                              is the hand-copied paragraph inside it — a seventh
                              implementation of the one sentence
                              `hostManagedModelCopy.ts` exists to keep in one
                              place. */}
                          {fixedByHost && (
                            <HostManagedModelNote
                              short
                              testId={`host-managed-config-${key}`}
                              className="mt-1"
                            />
                          )}
                        </div>
                        {/* Icon-only at both states, so it takes the 32×32 round
                            rung and an `aria-label` rather than a `min-w-[60px]`
                            box sized to hold a word it only sometimes shows. */}
                        <Button
                          onClick={() => handleSave(key)}
                          disabled={fixedByHost || !modifiedKeys.has(key) || saving === key}
                          title={fixedByHost ? HOST_MANAGED_MODEL_REASON : undefined}
                          variant="ghost"
                          shape="round"
                          aria-label={`Save ${getUiNames(key)}`}
                        >
                          {saving === key ? <Loader2 className="animate-spin" /> : <Save />}
                        </Button>
                      </div>
                    );
                  })
                )}
              </div>
            </div>

            {/* Footer roles, per the button table: the dismiss is `outline` and
                the quiet secondary action is `ghost`. ⚠ Do NOT touch either
                `onClick` — `setIsModalOpen(false)` deliberately differs from
                `handleModalClose` in whether unsaved edits are discarded. */}
            <DialogFooter className="gap-2">
              {modifiedKeys.size > 0 && (
                <Button onClick={handleReset} variant="ghost">
                  <RotateCcw />
                  Reset changes
                </Button>
              )}
              <Button onClick={() => setIsModalOpen(false)} variant="outline">
                Done
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </div>
    </div>
  );
}
