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
import {
  HOST_MANAGED_DESTINATION_REASON,
  HOST_MANAGED_MODEL_REASON,
} from '../../privacy/hostManagedModelCopy';
import { HostManagedModelNote, type HostManagedTopic } from '../../privacy/HostManagedModelNote';
import { isDestinationConfigKey } from '../destinationConfigKeys';
import { MODAL_SIZE } from '../../ModalShell';
import type { ConfigData, ConfigValue } from '../../../types/config';
import {
  PRIVACY_TIERS_KEY,
  PRIVACY_TIERS_RECORD_KEY,
  privacyTiersEnabledFromConfig,
  privacyTiersRecordFromConfig,
  type PrivacyTiersOrigin,
} from '../privacy/privacyTiers';
import { MIXING_POLICY_KEY } from '../../../utils/crossAffiliation';

/**
 * W2-PRV-8 — the keys this free-text editor must not offer.
 *
 * The master switch needs its typed confirmation and the mixing policy its
 * operating-system confirmation, so a Save here could only ever be refused;
 * and the record is a report the daemon composes on every read, which it shows
 * as an object: this editor rendered it as `[object Object]` with a Save that
 * wrote a line nothing reads. All three are shown read-only instead, with the
 * way to their real control.
 */
export const PRIVACY_CONFIG_KEYS: readonly string[] = [
  PRIVACY_TIERS_KEY,
  PRIVACY_TIERS_RECORD_KEY,
  MIXING_POLICY_KEY,
];

/** Where the privacy switch lives, as every surface names it. */
export const PRIVACY_SETTINGS_PATH = 'Settings > App > Privacy';

const ORIGIN_WORDS: Record<PrivacyTiersOrigin, string> = {
  settings: PRIVACY_SETTINGS_PATH,
  migration: 'An older configuration file',
  unrecorded: 'Outside the app',
  default: 'Never changed',
};

/** The daemon's refusal sentence, when a failed write carried one. */
function refusalText(error: unknown): string | null {
  const text = error instanceof Error ? error.message : typeof error === 'string' ? error : null;
  return text && text.trim() ? text.trim() : null;
}

/** A value this editor can show and save as text: a string, number or boolean. */
function isEditableValue(value: ConfigValue | undefined): boolean {
  return value === undefined || value === null || typeof value !== 'object';
}
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
   * key, and a browser-served daemon refuses two kinds of them: the capability
   * keys `is_capability_key` names (the model), and the keys that decide where
   * a provider sends its requests and key (W2-PRV-2, round 4:
   * `destinationConfigKeys.ts`, pinned to the daemon's list by a Rust test).
   * Greying out the whole page would be wrong about the great majority of it,
   * so the question is asked per row. See `utils/surface.ts` for the first
   * list and the drift risk it carries.
   */
  const hostManaged = isBrowserSurface();
  const hostTopic = (key: string): HostManagedTopic | null => {
    if (!hostManaged) return null;
    if (isHostManagedConfigKey(key)) return 'model';
    if (isDestinationConfigKey(key)) return 'destination';
    return null;
  };
  const isFixedByHost = (key: string) => hostTopic(key) !== null;

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
      // W2-PRV-8: the daemon's sentence is the answer, so it is the toast's
      // body. It used to sit only behind "Copy error".
      const reason = refusalText(error);
      toastError({
        title: 'Save failed',
        msg: reason
          ? `"${getUiNames(key)}" was not saved. ${reason}`
          : `Failed to save "${getUiNames(key)}"`,
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
        // Shown read-only above the list (W2-PRV-8).
        if (PRIVACY_CONFIG_KEYS.includes(key)) {
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
              <PrivacyConfigSummary
                config={typedConfig}
                onOpen={() => {
                  setIsModalOpen(false);
                  document
                    .querySelector('[data-privacy-panel]')
                    ?.scrollIntoView({ behavior: 'smooth', block: 'start' });
                }}
              />
              <div className="space-y-4">
                {configEntries.length === 0 ? (
                  <p className="text-text-muted">No configuration settings found.</p>
                ) : (
                  configEntries.map(([key, _value]) => {
                    const topic = hostTopic(key);
                    const fixedByHost = topic !== null;
                    return (
                      <div
                        key={key}
                        className="grid grid-cols-[minmax(0,200px)_1fr_auto] items-center gap-3"
                      >
                        <label className="text-label text-text-default" title={key}>
                          {getUiNames(key)}
                        </label>
                        <div className="min-w-0">
                          {/* A structured value is a report, not a setting this
                              field can round-trip: `String()` of it read
                              `[object Object]` (W2-PRV-8). */}
                          {!isEditableValue(configValues[key]) ? (
                            <code
                              data-testid={`config-readonly-${key}`}
                              className="block whitespace-pre-wrap break-all rounded-element bg-background-muted px-2 py-1.5 text-xs text-text-muted"
                            >
                              {JSON.stringify(configValues[key], null, 2)}
                            </code>
                          ) : (
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
                          )}
                          {/* The `topic &&` guard is load-bearing and stays: it
                              carries the per-key half (`isHostManagedConfigKey`
                              or `isDestinationConfigKey`), which the note itself
                              cannot know. What went is the hand-copied paragraph
                              inside it — a seventh implementation of the one
                              sentence `hostManagedModelCopy.ts` exists to keep
                              in one place. */}
                          {topic && (
                            <HostManagedModelNote
                              short
                              topic={topic}
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
                          disabled={
                            fixedByHost ||
                            !isEditableValue(configValues[key]) ||
                            !modifiedKeys.has(key) ||
                            saving === key
                          }
                          title={
                            topic === 'model'
                              ? HOST_MANAGED_MODEL_REASON
                              : topic === 'destination'
                                ? HOST_MANAGED_DESTINATION_REASON
                                : undefined
                          }
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

/**
 * W2-PRV-8 — the privacy settings this editor used to show as free text, as
 * what they are: the switch's state and its record, read-only, and the mixing
 * policy, with the one place each is changed.
 */
function PrivacyConfigSummary({ config, onOpen }: { config: ConfigData; onOpen: () => void }) {
  const hasAny = PRIVACY_CONFIG_KEYS.some((key) => config[key] !== undefined);
  if (!hasAny) return null;
  const enabled = privacyTiersEnabledFromConfig(config[PRIVACY_TIERS_KEY]);
  const record = privacyTiersRecordFromConfig(config[PRIVACY_TIERS_RECORD_KEY]);
  const mixing = config[MIXING_POLICY_KEY];
  const rows: [string, string][] = [
    ['Privacy tiers', enabled ? 'On' : 'Off'],
    ...(record
      ? ([
          ['Recorded as', record.enabled ? 'On' : 'Off'],
          ['Last changed', record.lastChange?.at || 'No change recorded'],
          ['Changed in', ORIGIN_WORDS[record.origin]],
        ] as [string, string][])
      : []),
    ...(typeof mixing === 'string' && mixing
      ? ([['Cross-institution mixing', mixing]] as [string, string][])
      : []),
  ];
  return (
    <div
      data-testid="config-privacy-summary"
      className="mb-4 rounded-container border border-border-subtle px-3 py-2.5"
    >
      <dl className="grid grid-cols-[minmax(0,200px)_1fr] gap-x-3 gap-y-1 text-sm">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-text-muted">{label}</dt>
            <dd className="min-w-0 text-text-default [overflow-wrap:anywhere]">{value}</dd>
          </div>
        ))}
      </dl>
      <p className="mt-2 text-supporting text-text-muted">
        These are changed in {PRIVACY_SETTINGS_PATH}, which asks you to confirm.{' '}
        <button
          type="button"
          onClick={onOpen}
          className="text-text-default underline underline-offset-2"
        >
          Go to Privacy
        </button>
      </p>
    </div>
  );
}
