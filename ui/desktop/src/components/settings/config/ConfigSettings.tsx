import { useState, useEffect, useMemo, useRef } from 'react';
import { Input } from '../../ui/input';
import { Button } from '../../ui/button';
import { SettingRow, SettingSection } from '../../ui/setting-row';
import { Spinner } from '../../ui/spinner';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { useConfig } from '../../ConfigContext';
import { useModelAndProvider } from '../../ModelAndProviderContext';
import { cn } from '../../../utils';
import { Save } from '../../icons/app-icons';
import { toastSuccess, toastError } from '../../../toasts';
import { getUiNames, providerPrefixes } from '../../../utils/configUtils';
import { isBrowserSurface, isHostManagedConfigKey } from '../../../utils/surface';
import { HostManagedModelNote, type HostManagedTopic } from '../../privacy/HostManagedModelNote';
import { isDestinationConfigKey } from '../destinationConfigKeys';
import { ModalShell } from '../../ModalShell';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
import { configCopy } from './copy';
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
export default function ConfigSettings() {
  const { config, upsert, refreshConfig } = useConfig();
  const { currentProvider: liveProvider, refreshCurrentModelAndProvider } = useModelAndProvider();
  const typedConfig = config as ConfigData;
  const [configValues, setConfigValues] = useState<ConfigData>({});
  const [modifiedKeys, setModifiedKeys] = useState<Set<string>>(new Set());
  const [saving, setSaving] = useState<string | null>(null);
  const [isModalOpen, setIsModalOpen] = useState(false);
  // The editor opens from state, with no Dialog.Trigger, so Radix would leave focus on <body> when
  // it closes. The Edit button takes it back.
  const editButtonRef = useRef<HTMLButtonElement>(null);
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
        title: configCopy.savedTitle,
        msg: configCopy.saved(getUiNames(key)),
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
        title: configCopy.saveFailedTitle,
        msg: reason
          ? configCopy.notSaved(getUiNames(key), reason)
          : configCopy.saveFailed(getUiNames(key)),
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
      title: configCopy.resetTitle,
      msg: configCopy.reset,
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
    <SettingSection id={SETTINGS_SECTION_IDS.configuration} title={configCopy.section}>
      <SettingRow
        label={configCopy.row}
        help={configCopy.help}
        value={currentProvider || undefined}
        valueMono
        controlId="configuration-edit"
        data-testid="config-settings-row"
      >
        <Button
          ref={editButtonRef}
          type="button"
          variant="secondary"
          size="sm"
          aria-label={configCopy.editName}
          onClick={() => setIsModalOpen(true)}
        >
          {configCopy.edit}
        </Button>
      </SettingRow>

      <ModalShell
        open={isModalOpen}
        onOpenChange={handleModalClose}
        size="lg"
        scrollBody
        // A form: Escape and the × leave, a stray click on the backdrop does not (it would throw
        // away edits that are typed but not yet saved).
        purpose="form"
        title={configCopy.dialogTitle}
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          editButtonRef.current?.focus();
        }}
        // Footer roles: the dismiss is `secondary` and the quiet action is `ghost`.
        // ⚠ Do NOT touch either `onClick` — `setIsModalOpen(false)` deliberately differs from
        // `handleModalClose` in whether unsaved edits are discarded.
        footer={
          <>
            {modifiedKeys.size > 0 && (
              <Button onClick={handleReset} variant="ghost">
                {configCopy.resetChanges}
              </Button>
            )}
            <Button onClick={() => setIsModalOpen(false)} variant="secondary">
              {configCopy.done}
            </Button>
          </>
        }
      >
        {/* The shell's header and footer own the body's top and bottom gutters only when the body
            is bare text; this body starts with a box and ends with a field, so it pads itself. */}
        <div className="py-3">
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
              <p className="text-body text-text-muted">{configCopy.empty}</p>
            ) : (
              configEntries.map(([key, _value]) => {
                const topic = hostTopic(key);
                const fixedByHost = topic !== null;
                const fieldId = `config-field-${key}`;
                const uiName = getUiNames(key);
                // ⚠ Exactly one button per row, and the row is this `div.grid`: two browser-surface
                // tests find a row with `closest('.grid')` and take its only button. No InfoTip here.
                return (
                  <div
                    key={key}
                    className="grid grid-cols-[minmax(0,200px)_1fr_auto] items-center gap-3"
                  >
                    {/* The key itself is the label's tooltip, in the mono face: it is the name a
                      person looks for in the file. It was a native `title`, which opens on no
                      keyboard and looks like the OS. */}
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <label
                          htmlFor={fieldId}
                          className="min-w-0 break-words text-label text-text-default"
                        >
                          {uiName}
                        </label>
                      </TooltipTrigger>
                      <TooltipContent>
                        <span className="font-mono">{key}</span>
                      </TooltipContent>
                    </Tooltip>
                    <div className="min-w-0">
                      {/* A structured value is a report, not a setting this
                        field can round-trip: `String()` of it read
                        `[object Object]` (W2-PRV-8). */}
                      {!isEditableValue(configValues[key]) ? (
                        <code
                          data-testid={`config-readonly-${key}`}
                          className="block whitespace-pre-wrap break-all rounded-element bg-background-muted px-2 py-1.5 font-mono text-supporting text-text-muted"
                        >
                          {JSON.stringify(configValues[key], null, 2)}
                        </code>
                      ) : (
                        <Input
                          id={fieldId}
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
                          // from it. The placeholder went too: the label names
                          // the field.
                          className={cn(modifiedKeys.has(key) && 'border-border-info')}
                        />
                      )}
                      {/* The `topic &&` guard is load-bearing and stays: it
                        carries the per-key half (`isHostManagedConfigKey`
                        or `isDestinationConfigKey`), which the note itself
                        cannot know. What went is the hand-copied paragraph
                        inside it — a seventh implementation of the one
                        sentence `hostManagedModelCopy.ts` exists to keep
                        in one place. It is also why the Save button below
                        has no `title`: this note, visible, is the reason. */}
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
                      variant="ghost"
                      shape="round"
                      aria-label={configCopy.saveLabel(uiName)}
                    >
                      {saving === key ? <Spinner /> : <Save />}
                    </Button>
                  </div>
                );
              })
            )}
          </div>
        </div>
      </ModalShell>
    </SettingSection>
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
      {/* The first column is the fields' 200px less the box's 1px border and 12px padding, so a value
          starts exactly where the inputs below it do (at 13px it sat to their right). */}
      <dl className="grid grid-cols-[minmax(0,187px)_1fr] gap-x-3 gap-y-1 text-body">
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
