import { useState } from 'react';
import { Check, ChevronDown, Loader2 } from '../icons/app-icons';
import { Button } from '../ui/button';
import { PermissionCheckButton } from './PermissionCheckButton';
import { copilotSetup, type CopilotRuntime } from './copilotApi';

const RUNTIME_STATES: Record<string, string> = {
  ready: 'Runtime ready',
  missing_runtime: 'Bundled runtime missing',
  incompatible_runtime: 'Runtime incompatible',
  desktop_unavailable: 'Desktop unavailable',
  probe_pending: 'Runtime found — setup not checked',
  os_permission_required: 'OS permission required',
  unsupported_environment: 'Unsupported desktop environment',
  missing_dependency: 'System dependency missing',
  probe_failed: 'Could not check the native runtime',
};

export type RuntimeVerdict = 'ready' | 'blocked' | 'unverified' | 'unavailable';

// A failed runtime probe is not evidence that an OS permission was denied.
export function runtimeVerdict(runtime: CopilotRuntime): RuntimeVerdict {
  if (
    [
      'missing_runtime',
      'incompatible_runtime',
      'probe_failed',
      'desktop_unavailable',
      'unsupported_environment',
      'missing_dependency',
    ].includes(runtime.status)
  )
    return 'unavailable';
  const permissions = runtime.permissions;
  if (typeof permissions !== 'object' || permissions === null) return 'unverified';
  const granted = [permissions.accessibility, permissions.screen_recording];
  if (granted.some((value) => value === false)) return 'blocked';
  if (granted.some((value) => value !== true)) return 'unverified';
  return runtime.status === 'ready' ? 'ready' : 'unverified';
}

export function CopilotRuntimeDetails({ runtime }: { runtime: CopilotRuntime }) {
  const [settingsError, setSettingsError] = useState('');
  const localMac =
    window.electron?.platform === 'darwin' &&
    window.electron?.getConfig?.().BIOROUTER_LOCAL_BACKEND === true &&
    /darwin/.test(runtime.target ?? '') &&
    Boolean(window.electron?.openCopilotPermissionSettings);
  const openSettings = async (permission: 'accessibility' | 'screen_recording') => {
    setSettingsError('');
    try {
      await window.electron.openCopilotPermissionSettings(permission);
    } catch (error) {
      setSettingsError(
        error instanceof Error
          ? error.message
          : 'Could not open System Settings. Open Privacy & Security manually.'
      );
    }
  };
  const permissionSummary =
    typeof runtime.permissions === 'string'
      ? runtime.permissions === 'unknown'
        ? 'Not checked. Runtime availability does not grant operating-system access.'
        : runtime.permissions
      : Object.entries(runtime.permissions)
          .map(
            ([name, granted]) =>
              `${name === 'screen_recording' ? 'Screen Recording' : 'Accessibility'}: ${granted === true ? 'allowed' : granted === false ? 'not allowed' : 'not checked'}`
          )
          .join(' · ');
  const target = runtime.target ?? '';
  const verdict = runtimeVerdict(runtime);
  return (
    <div className="space-y-1 text-supporting text-text-muted">
      <p>{RUNTIME_STATES[runtime.status] ?? runtime.status}</p>
      {runtime.error && <p className="break-words">{runtime.error}</p>}
      {runtime.host && <p>Backend computer: {runtime.host}</p>}
      {runtime.runtime_version && (
        <p>
          Runtime version: {runtime.runtime_version}
          {runtime.target ? ` · ${runtime.target}` : ''}
        </p>
      )}
      <p>OS permissions: {permissionSummary || 'Not reported by this environment.'}</p>
      {/* ⚠ NAME THE BINARY. macOS keys Screen Recording to a particular executable
          PATH, not to the signing identity, so a machine holding more than one copy
          of the helper — an installed Biorouter, a packaged build, a source build —
          needs each copy granted separately. Without this line the panel says
          "enable Screen Recording for BioRouter Computer Use" to someone who has
          already done exactly that, for a different copy, and there is nothing on
          screen to tell them so. Measured 2026-09-23: three copies on one machine,
          two reporting `screen_recording: true` and the source build reporting
          false, with an identical bundle id and the same Developer ID. */}
      {runtime.executable && verdict !== 'ready' && (
        <p className="break-all">
          Grant these to this exact copy: <code>{runtime.executable}</code>
        </p>
      )}
      {runtime.message && <p>{runtime.message}</p>}
      {runtime.status === 'missing_runtime' || runtime.status === 'incompatible_runtime' ? (
        <p>
          Install or repair the matching Biorouter package on the backend computer, then check
          again.
        </p>
      ) : null}
      {runtime.status === 'desktop_unavailable' && (
        <p>Run Biorouter in a signed-in desktop session on the backend computer.</p>
      )}
      {runtime.status === 'probe_failed' && (
        <p>
          The permission check failed; this does not mean access was denied. Restart Biorouter on
          the backend computer and check again. If it still fails, repair the matching Biorouter
          package.
        </p>
      )}
      {/* The remediation below used to render unconditionally, on `target` alone.
          That is why a fully granted machine was told to "Review Accessibility
          and Screen Recording ..." directly under "Native desktop access is
          ready." A satisfied runtime now says so instead. */}
      {verdict === 'ready' ? (
        <p className="flex items-start gap-1.5 text-text-success">
          <Check className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
          <span>
            Accessibility and Screen Recording are allowed on the backend computer. Nothing further
            to set up.
          </span>
        </p>
      ) : ['missing_runtime', 'incompatible_runtime', 'probe_failed'].includes(
          runtime.status
        ) ? null : /darwin/.test(target) ? (
        <p>
          Review Accessibility and Screen Recording in System Settings → Privacy &amp; Security on
          the backend computer, then check again.
        </p>
      ) : target.startsWith('win32-') || target.includes('windows') ? (
        <p>
          Use a signed-in interactive desktop. Secure desktops and elevation prompts cannot be
          controlled.
        </p>
      ) : /linux/.test(target) ? (
        <p>
          Enable accessibility in the backend desktop session and install the system dependencies
          reported above. Desktop portals may request additional permissions.
        </p>
      ) : (
        <p>
          Grant the operating-system permissions requested on the backend computer, then check
          again.
        </p>
      )}
      {verdict !== 'ready' &&
        (verdict !== 'unavailable' || runtime.status === 'probe_failed') &&
        /darwin/.test(target) && (
          <div className="space-y-2">
            <p>
              {runtime.message?.includes('System Settings')
                ? 'Use the app names in the permission details above.'
                : 'For the bundled app, enable Accessibility for BioRouter Computer Use and Screen Recording for Biorouter.'}{' '}
              If macOS asks you to quit and reopen it, do so, then use Check again. Checking
              permissions does not grant them.
            </p>
            {localMac ? (
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => void openSettings('accessibility')}
                >
                  Open Accessibility settings
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => void openSettings('screen_recording')}
                >
                  Open Screen Recording settings
                </Button>
              </div>
            ) : (
              <p>
                Make these changes on the backend computer. If it is remote, changing permissions on
                this device will not change the backend's access.
              </p>
            )}
            {settingsError && <p role="alert">{settingsError}</p>}
          </div>
        )}
      {runtime.development_override && (
        <p>
          Using an explicitly configured development runtime (BIOROUTER_COMPUTER_USE_DIR). Its
          operating-system permissions are separate from the installed app&apos;s.
        </p>
      )}
    </div>
  );
}

export function CopilotSetup() {
  const [runtime, setRuntime] = useState<CopilotRuntime>();
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [open, setOpen] = useState(false);
  const [checked, setChecked] = useState(false);

  const check = async () => {
    setLoading(true);
    setError('');
    try {
      const next = await copilotSetup();
      setRuntime(next);
      setChecked(true);
      // Opened HERE, not in the click handler: a first press that FAILS must not
      // leave `open` true, or the next successful press flips it straight back
      // to closed and the panel stays hidden a click out of phase.
      setOpen(true);
    } catch (failure) {
      setError(
        failure instanceof Error
          ? failure.message
          : 'Could not read Biorouter Copilot setup. Check the backend connection and try again.'
      );
      setChecked(false);
    } finally {
      setLoading(false);
    }
  };

  // Showing a result already fetched and asking the backend again are different
  // acts, so they are different controls. The trigger only shows/hides what is
  // already known -- except on the very first press, which has nothing to show
  // yet and so opens AND checks. "Check again", inside the panel, is the only
  // thing that re-reads the backend.
  const toggle = () => {
    // The first press has nothing to show yet, so it checks and lets check()
    // open the panel. Every later press only shows or hides what is known.
    if (!runtime) {
      if (!loading) void check();
      return;
    }
    setOpen((current) => !current);
  };
  const expanded = open && Boolean(runtime);

  return (
    <div className="mt-2 space-y-2">
      <Button
        size="sm"
        variant="outline"
        disabled={loading}
        aria-busy={loading}
        aria-expanded={expanded}
        aria-controls="copilot-setup-details"
        onClick={toggle}
      >
        {loading && <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />}
        {loading
          ? 'Checking…'
          : !runtime
            ? 'Check Biorouter Copilot setup'
            : open
              ? 'Hide Biorouter Copilot setup'
              : 'Show Biorouter Copilot setup'}
        {runtime && !loading && (
          <ChevronDown
            className={`size-3.5 shrink-0 transition-transform ${open ? 'rotate-180' : 'rotate-0'}`}
            aria-hidden="true"
          />
        )}
      </Button>
      {expanded && runtime && (
        <div id="copilot-setup-details" className="space-y-2">
          <CopilotRuntimeDetails runtime={runtime} />
          <PermissionCheckButton
            label="Check again"
            checking={loading}
            checked={checked}
            verdict={runtimeVerdict(runtime)}
            onCheck={() => void check()}
          />
        </div>
      )}
      {error && (
        <p role="alert" className="text-supporting text-text-danger">
          {error}
        </p>
      )}
      <p className="text-supporting text-text-muted">
        Allow each task from its chat. The active indicator and Stop control stay above the
        composer.
      </p>
    </div>
  );
}
