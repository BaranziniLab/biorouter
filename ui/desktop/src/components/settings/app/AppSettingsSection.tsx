import { useState, useEffect, useRef } from 'react';
import { Switch } from '../../ui/switch';
import { Button } from '../../ui/button';
import { Settings } from '../../icons/app-icons';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../ui/dialog';
import UpdateSection from './UpdateSection';
import UsageSection from '../usage/UsageSection';
import ResetPanel from './ResetPanel';

import { COST_TRACKING_ENABLED, UPDATES_ENABLED } from '../../../updates';
import ThemeSelector from '../../BioRouterSidebar/ThemeSelector';
import ThemeFamilySelector from '../../BioRouterSidebar/ThemeFamilySelector';
import BlockLogoBlack from './icons/block-lockup_black.png';
import BlockLogoWhite from './icons/block-lockup_white.png';
import { useResolvedTheme } from '../../../contexts/ThemeContext';
import { useTransientFlag } from '../../../hooks/useTransientFlag';

interface AppSettingsSectionProps {
  scrollToSection?: string;
}

export default function AppSettingsSection({ scrollToSection }: AppSettingsSectionProps) {
  const [menuBarIconEnabled, setMenuBarIconEnabled] = useState(true);
  const [dockIconEnabled, setDockIconEnabled] = useState(true);
  const [wakelockEnabled, setWakelockEnabled] = useState(true);
  const [isMacOS, setIsMacOS] = useState(false);
  // The dock switch is held down for a second while the OS applies the change.
  const [isDockSwitchDisabled, disableDockSwitch] = useTransientFlag(1000);
  const [showNotificationModal, setShowNotificationModal] = useState(false);
  const [showPricing, setShowPricing] = useState(true);
  // The app already resolves light/dark once, in `ThemeContext`. This file kept
  // its own `MutationObserver` on `<html>`'s class list to answer the same
  // question — a second source of truth that could disagree with the first, for
  // one logo swap.
  const mode = useResolvedTheme();
  const [usageVersion, setUsageVersion] = useState(0);
  const updateSectionRef = useRef<HTMLDivElement>(null);

  const shouldShowUpdates = !window.appConfig.get('BIOROUTER_VERSION');

  useEffect(() => {
    setIsMacOS(window.electron.platform === 'darwin');
  }, []);

  useEffect(() => {
    const stored = localStorage.getItem('show_pricing');
    setShowPricing(stored !== 'false');
  }, []);

  useEffect(() => {
    if (scrollToSection === 'update' && updateSectionRef.current) {
      setTimeout(() => {
        updateSectionRef.current?.scrollIntoView({ behavior: 'smooth', block: 'center' });
      }, 100);
    }
  }, [scrollToSection]);

  useEffect(() => {
    window.electron.getMenuBarIconState().then((enabled) => {
      setMenuBarIconEnabled(enabled);
    });

    window.electron.getWakelockState().then((enabled) => {
      setWakelockEnabled(enabled);
    });

    if (isMacOS) {
      window.electron.getDockIconState().then((enabled) => {
        setDockIconEnabled(enabled);
      });
    }
  }, [isMacOS]);

  const handleMenuBarIconToggle = async () => {
    const newState = !menuBarIconEnabled;
    if (!newState && !dockIconEnabled && isMacOS) {
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
    const newState = !dockIconEnabled;
    if (!newState && !menuBarIconEnabled) {
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

  // A fragment, not a `pb-8` wrapper: these five sections are siblings of
  // Privacy's and Workspace's, so the 10px `.biorouter-settings-section +
  // .biorouter-settings-section` adjacency fires across all of them. The tail
  // spacer lives once, on the App tab's own wrapper in `SettingsView`.
  return (
    <>
      {/* Appearance */}
      <div className="biorouter-settings-section">
        <div className="biorouter-settings-section-header">
          <h2 className="text-caps text-text-muted">Appearance</h2>
        </div>
        <div className="biorouter-settings-list">
          <div className="biorouter-settings-row flex min-w-0 items-center justify-between gap-3 px-3 py-2.5">
            <div className="min-w-0 flex-1">
              <p className="text-label text-text-default">Notifications</p>
              <p className="mt-0.5 max-w-md text-supporting text-text-muted">
                Notifications are managed by your OS.{' '}
                {/* A real `<Button variant="link">`, not a `<span onClick>` — it
                    was unreachable by keyboard and announced as text. The three
                    neutralisers are required rather than decorative: the cva base
                    is `text-label` and `link` only underlines on hover, so
                    without them a 14px semibold word lands mid-sentence and the
                    control's only affordance disappears until you point at it. */}
                <Button
                  variant="link"
                  className="h-auto p-0 align-baseline text-supporting font-normal underline"
                  onClick={() => setShowNotificationModal(true)}
                >
                  Configuration guide
                </Button>
              </p>
            </div>
            <Button
              variant="secondary"
              onClick={async () => {
                try {
                  await window.electron.openNotificationsSettings();
                } catch (error) {
                  console.error('Failed to open notification settings:', error);
                }
              }}
            >
              <Settings />
              Open settings
            </Button>
          </div>

          <div className="biorouter-settings-row flex items-center justify-between px-3 py-2.5">
            <div className="min-w-0 flex-1">
              <p className="text-label text-text-default">Menu bar icon</p>
              <p className="mt-0.5 max-w-md text-supporting text-text-muted">
                Show Biorouter in the menu bar
              </p>
            </div>
            <Switch
              checked={menuBarIconEnabled}
              onCheckedChange={handleMenuBarIconToggle}
              variant="mono"
              aria-label="Menu bar icon"
            />
          </div>

          {isMacOS && (
            <div className="biorouter-settings-row flex items-center justify-between px-3 py-2.5">
              <div className="min-w-0 flex-1">
                <p className="text-label text-text-default">Dock icon</p>
                <p className="mt-0.5 max-w-md text-supporting text-text-muted">
                  Show Biorouter in the dock
                </p>
              </div>
              <Switch
                disabled={isDockSwitchDisabled}
                checked={dockIconEnabled}
                onCheckedChange={handleDockIconToggle}
                variant="mono"
                aria-label="Dock icon"
              />
            </div>
          )}

          <div className="biorouter-settings-row flex items-center justify-between px-3 py-2.5">
            <div className="min-w-0 flex-1">
              <p className="text-label text-text-default">Prevent sleep</p>
              <p className="mt-0.5 max-w-md text-supporting text-text-muted">
                Keep your computer awake while Biorouter is running a task (screen can still lock)
              </p>
            </div>
            <Switch
              checked={wakelockEnabled}
              onCheckedChange={handleWakelockToggle}
              variant="mono"
              aria-label="Prevent sleep"
            />
          </div>

          {COST_TRACKING_ENABLED && (
            <div className="biorouter-settings-row flex items-center justify-between px-3 py-2.5">
              <div className="min-w-0 flex-1">
                <p className="text-label text-text-default">Cost tracking</p>
                <p className="mt-0.5 max-w-md text-supporting text-text-muted">
                  Show model pricing and usage costs
                </p>
              </div>
              <Switch
                checked={showPricing}
                onCheckedChange={handleShowPricingToggle}
                variant="mono"
                aria-label="Cost tracking"
              />
            </div>
          )}
        </div>
      </div>

      {/* Theme. Palette and Mode are two SECTIONS, not two labelled sub-blocks
          inside one: a `text-xs` paragraph acting as a heading was a fourth
          heading style on a tab where every other group is a `text-caps`
          section label, and the `flex flex-col gap-4` wrapper it lived in
          forked this one group off the section rhythm. */}
      <div className="biorouter-settings-section">
        <div className="biorouter-settings-section-header">
          <h2 className="text-caps text-text-muted mb-1">Palette</h2>
          <p className="text-supporting text-text-muted">Change how Biorouter looks</p>
        </div>
        <div className="biorouter-settings-control-strip">
          <ThemeFamilySelector className="w-auto" horizontal />
        </div>
      </div>

      <div className="biorouter-settings-section">
        <div className="biorouter-settings-section-header">
          <h2 className="text-caps text-text-muted">Mode</h2>
        </div>
        <div className="biorouter-settings-control-strip">
          <ThemeSelector className="w-auto" horizontal />
        </div>
      </div>

      {/* Usage — accumulated (billed) tokens + cost, month-to-date vs budget */}
      {COST_TRACKING_ENABLED && showPricing && <UsageSection key={usageVersion} />}

      <ResetPanel
        onReset={(categories) => {
          if (categories.includes('history')) setUsageVersion((version) => version + 1);
        }}
      />

      {/* Help & Feedback */}
      <div className="biorouter-settings-section">
        <div className="biorouter-settings-section-header">
          <h2 className="text-caps text-text-muted mb-1">Help &amp; Feedback</h2>
          <p className="text-supporting text-text-muted">
            Report a problem, or ask for something Biorouter does not do yet
          </p>
        </div>
        <div className="biorouter-settings-control-strip">
          <Button
            onClick={() => {
              window.open(
                'https://github.com/BaranziniLab/biorouter/issues/new?template=bug_report.md',
                '_blank'
              );
            }}
            variant="secondary"
          >
            Report a bug
          </Button>
          <Button
            onClick={() => {
              window.open(
                'https://github.com/BaranziniLab/biorouter/issues/new?template=feature_request.md',
                '_blank'
              );
            }}
            variant="secondary"
          >
            Request a feature
          </Button>
        </div>
      </div>

      {/* Version */}
      {!shouldShowUpdates && (
        <div className="biorouter-settings-section">
          <div className="biorouter-settings-section-header">
            <h2 className="text-caps text-text-muted">Version</h2>
          </div>
          {/* Not a `.biorouter-settings-control-strip`: the strip is a BUTTON
              row, and wrapping anything else in it shrink-wraps that thing to
              content width. */}
          <div className="flex items-center gap-3">
            <img
              src={mode === 'dark' ? BlockLogoWhite : BlockLogoBlack}
              alt="Block Logo"
              className="h-8 w-auto"
            />
            <span className="text-display font-mono text-text-default">
              {String(window.appConfig.get('BIOROUTER_VERSION') || 'Development')}
            </span>
          </div>
        </div>
      )}

      {/* Updates */}
      {UPDATES_ENABLED && shouldShowUpdates && (
        <div ref={updateSectionRef} className="biorouter-settings-section">
          <div className="biorouter-settings-section-header">
            <h2 className="text-caps text-text-muted mb-1">Updates</h2>
            <p className="text-supporting text-text-muted">Check for and install updates</p>
          </div>
          {/* `UpdateSection` is a multi-row panel, not a button row — the strip
              was shrink-wrapping it to its content width. */}
          <UpdateSection />
        </div>
      )}

      {/* Notification Instructions Modal */}
      <Dialog
        open={showNotificationModal}
        onOpenChange={(open) => !open && setShowNotificationModal(false)}
      >
        <DialogContent className="sm:max-w-[500px]">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              {/* `text-iconStandard` is not a token — it had no effect and no
                  definition. The two dialog titles now agree on 20px. */}
              <Settings size={20} />
              How to Enable Notifications
            </DialogTitle>
          </DialogHeader>

          <div className="py-4">
            {isMacOS ? (
              <div className="space-y-4">
                <DialogDescription className="text-text-default">
                  To enable notifications on macOS:
                </DialogDescription>
                <ol className="list-decimal pl-5 space-y-2">
                  <li>Open System Preferences</li>
                  <li>Click on Notifications</li>
                  <li>Find and select Biorouter in the app list</li>
                  <li>Enable notifications and adjust settings as desired</li>
                </ol>
              </div>
            ) : (
              <div className="space-y-4">
                <DialogDescription className="text-text-default">
                  To enable notifications on Windows:
                </DialogDescription>
                <ol className="list-decimal pl-5 space-y-2">
                  <li>Open Settings</li>
                  <li>Go to System &gt; Notifications</li>
                  <li>Find and select Biorouter in the app list</li>
                  <li>Toggle notifications on and adjust settings as desired</li>
                </ol>
              </div>
            )}
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={() => setShowNotificationModal(false)}>
              Close
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
