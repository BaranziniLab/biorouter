import { useState, useEffect } from 'react';
import { Switch } from '../../ui/switch';
import { StoredSwitch } from '../StoredSwitch';
import { Button } from '../../ui/button';
import { SettingRow, SettingSection } from '../../ui/setting-row';
import UpdateSection from './UpdateSection';
import UsageSection from '../usage/UsageSection';
import ResetPanel from './ResetPanel';
import FontSizeSelector from './FontSizeSelector';
import { NeverOpenTabsRow } from './WorkspaceSettingsSection';
import { COST_TRACKING_ENABLED, UPDATES_ENABLED } from '../../../updates';
import ThemeSelector from '../../BioRouterSidebar/ThemeSelector';
import ThemeFamilySelector from '../../BioRouterSidebar/ThemeFamilySelector';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
import { useTransientFlag } from '../../../hooks/useTransientFlag';
import { aboutCopy, appearanceCopy, generalCopy } from './copy';

const BUG_REPORT_URL =
  'https://github.com/BaranziniLab/biorouter/issues/new?template=bug_report.md';
const FEATURE_REQUEST_URL =
  'https://github.com/BaranziniLab/biorouter/issues/new?template=feature_request.md';

/** Whether costs are shown; on unless the person turned them off (and on when storage is closed). */
function readShowPricing(): boolean {
  try {
    return localStorage.getItem('show_pricing') !== 'false';
  } catch {
    return true;
  }
}

/**
 * Settings > App, after Configuration and Privacy (the operator's order, kept by `SettingsView`):
 * General, Appearance, Usage, About, Danger zone (spec §3.13).
 *
 * Every row is one `SettingRow`: a label (which is also the control's accessible name, so a
 * person driving the app by voice says what they read), an optional InfoTip, and ONE control
 * at the trailing edge. No row carries a paragraph.
 *
 * Returns a fragment: these sections are siblings of Configuration's and Privacy's, so the
 * `.biorouter-settings-section + .biorouter-settings-section` adjacency fires across all of
 * them. The tail spacer lives once, on the App tab's own wrapper in `SettingsView`.
 */
export default function AppSettingsSection() {
  // `null` until the main process has answered. These three switches hold their place unpainted
  // until then (`StoredSwitch`): a placeholder `true` flashed the coral ON track on every open for
  // a person who had turned the setting off (vision V4-01).
  const [menuBarIconEnabled, setMenuBarIconEnabled] = useState<boolean | null>(null);
  const [dockIconEnabled, setDockIconEnabled] = useState<boolean | null>(null);
  const [wakelockEnabled, setWakelockEnabled] = useState<boolean | null>(null);
  // Known on the first render: the platform is synchronous, and a row that appears one frame
  // later (Show in Dock) shifts every row below it.
  const [isMacOS] = useState(() => window.electron.platform === 'darwin');
  // The dock switch is held down for a second while the OS applies the change.
  const [isDockSwitchDisabled, disableDockSwitch] = useTransientFlag(1000);
  // Read before the first paint, so Usage does not appear and vanish for a person who hid it.
  const [showPricing, setShowPricing] = useState(readShowPricing);
  const [usageVersion, setUsageVersion] = useState(0);

  const pinnedVersion = window.appConfig.get('BIOROUTER_VERSION');
  const shouldShowUpdates = !pinnedVersion;

  useEffect(() => {
    // A state that cannot be read leaves the setting at its default, which is on for all three,
    // rather than a switch that never appears.
    const read = (ask: () => Promise<boolean>, set: (enabled: boolean) => void) => {
      ask().then(set, () => set(true));
    };
    read(() => window.electron.getMenuBarIconState(), setMenuBarIconEnabled);
    read(() => window.electron.getWakelockState(), setWakelockEnabled);
    if (isMacOS) read(() => window.electron.getDockIconState(), setDockIconEnabled);
  }, [isMacOS]);

  const handleMenuBarIconToggle = async () => {
    if (menuBarIconEnabled === null) return;
    const newState = !menuBarIconEnabled;
    // At least one of the two must stay on, or the app has no visible handle.
    if (!newState && dockIconEnabled === false && isMacOS) {
      const success = await window.electron.setDockIcon(true);
      if (success) {
        setDockIconEnabled(true);
      }
    }
    const success = await window.electron.setMenuBarIcon(newState);
    if (success) {
      setMenuBarIconEnabled(newState);
    }
  };

  const handleDockIconToggle = async () => {
    if (dockIconEnabled === null) return;
    const newState = !dockIconEnabled;
    if (!newState && menuBarIconEnabled === false) {
      const success = await window.electron.setMenuBarIcon(true);
      if (success) {
        setMenuBarIconEnabled(true);
      }
    }
    disableDockSwitch();
    const success = await window.electron.setDockIcon(newState);
    if (success) {
      setDockIconEnabled(newState);
    }
  };

  const handleWakelockToggle = async () => {
    if (wakelockEnabled === null) return;
    const newState = !wakelockEnabled;
    const success = await window.electron.setWakelock(newState);
    if (success) {
      setWakelockEnabled(newState);
    }
  };

  const handleShowPricingToggle = (checked: boolean) => {
    setShowPricing(checked);
    localStorage.setItem('show_pricing', String(checked));
    window.dispatchEvent(new CustomEvent('storage'));
  };

  const openNotificationsLabel = isMacOS
    ? generalCopy.openNotificationsMac
    : generalCopy.openNotificationsOther;

  return (
    <>
      <SettingSection id={SETTINGS_SECTION_IDS.general} title={generalCopy.section}>
        <SettingRow label={generalCopy.notifications} help={generalCopy.notificationsHelp}>
          {/* The visible words are the name (label in name), not the row's label. */}
          <Button
            variant="secondary"
            size="sm"
            aria-label={openNotificationsLabel}
            onClick={async () => {
              try {
                await window.electron.openNotificationsSettings();
              } catch (error) {
                console.error('Failed to open notification settings:', error);
              }
            }}
          >
            {openNotificationsLabel}
          </Button>
        </SettingRow>

        <SettingRow label={generalCopy.menuBar}>
          <StoredSwitch checked={menuBarIconEnabled} onCheckedChange={handleMenuBarIconToggle} />
        </SettingRow>

        {isMacOS && (
          <SettingRow label={generalCopy.dock}>
            <StoredSwitch
              disabled={isDockSwitchDisabled}
              checked={dockIconEnabled}
              onCheckedChange={handleDockIconToggle}
            />
          </SettingRow>
        )}

        <SettingRow label={generalCopy.preventSleep} help={generalCopy.preventSleepHelp}>
          <StoredSwitch checked={wakelockEnabled} onCheckedChange={handleWakelockToggle} />
        </SettingRow>

        <NeverOpenTabsRow />

        {COST_TRACKING_ENABLED && (
          <SettingRow label={generalCopy.showCosts} help={generalCopy.showCostsHelp}>
            <Switch checked={showPricing} onCheckedChange={handleShowPricingToggle} />
          </SettingRow>
        )}
      </SettingSection>

      <SettingSection id={SETTINGS_SECTION_IDS.appearance} title={appearanceCopy.section}>
        <SettingRow label={appearanceCopy.theme}>
          <ThemeSelector />
        </SettingRow>
        <SettingRow label={appearanceCopy.palette}>
          <ThemeFamilySelector />
        </SettingRow>
        <SettingRow label={appearanceCopy.textSize}>
          <FontSizeSelector />
        </SettingRow>
      </SettingSection>

      {/*
        Usage: accumulated (billed) tokens and cost. WS-USAGE owns the section. It stays mounted
        when Show costs goes off and only drops its dollar figures: unmounting it took 110px out of
        the page, which clamped the scroll and moved every row under the pointer, and it hid token
        counts and turns that are not costs (vision V4-02).
      */}
      {COST_TRACKING_ENABLED && <UsageSection key={usageVersion} showCosts={showPricing} />}

      <SettingSection id={SETTINGS_SECTION_IDS.about} title={aboutCopy.section}>
        {UPDATES_ENABLED && shouldShowUpdates ? (
          <UpdateSection />
        ) : (
          // A build pinned to a version (or a browser-served page) has no updater.
          <SettingRow
            label={aboutCopy.version}
            value={String(pinnedVersion || aboutCopy.development)}
            valueMono
          />
        )}
        <SettingRow label={aboutCopy.feedback}>
          <div role="group" className="flex items-center gap-1">
            <Button variant="ghost" size="sm" onClick={() => window.open(BUG_REPORT_URL, '_blank')}>
              {aboutCopy.reportBug}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => window.open(FEATURE_REQUEST_URL, '_blank')}
            >
              {aboutCopy.requestFeature}
            </Button>
          </div>
        </SettingRow>
      </SettingSection>

      <ResetPanel
        onReset={(categories) => {
          if (categories.includes('history')) setUsageVersion((version) => version + 1);
        }}
      />
    </>
  );
}
