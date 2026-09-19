import { useState } from 'react';
import { Check, ChevronDown, Loader2 } from '../icons/app-icons';
import { Button } from '../ui/button';
import { PermissionCheckButton } from './PermissionCheckButton';
import { computerUseSetup, type ComputerUseRuntime } from './computerUseApi';

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

export type RuntimeVerdict = 'ready' | 'blocked' | 'unverified';

/**
 * Whether this runtime needs anything from the user. Pure, so the branch is
 * testable without mounting.
 *
 * `ready` demands BOTH permissions explicitly true AND a ready status, so a
 * `desktop_unavailable` runtime with every permission granted is still blocked:
 * the remediation text below is what tells the user what to do about it, and
 * suppressing it there would leave a dead end.
 */
export function runtimeVerdict(runtime: ComputerUseRuntime): RuntimeVerdict {
  const permissions = runtime.permissions;
  if (typeof permissions !== 'object' || permissions === null) return 'unverified';
  const granted = [permissions.accessibility, permissions.screen_recording];
  if (granted.some((value) => value === false)) return 'blocked';
  if (granted.some((value) => value !== true)) return 'unverified';
  return runtime.status === 'ready' ? 'ready' : 'blocked';
}

export function ComputerUseRuntimeDetails({ runtime }: { runtime: ComputerUseRuntime }) {
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
      ) : /darwin/.test(target) ? (
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
      {runtime.development_override && <p>Using an explicitly configured development runtime.</p>}
    </div>
  );
}

export function ComputerUseSetup() {
  const [runtime, setRuntime] = useState<ComputerUseRuntime>();
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [open, setOpen] = useState(false);
  const [checked, setChecked] = useState(false);

  const check = async () => {
    setLoading(true);
    setError('');
    try {
      const next = await computerUseSetup();
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
          : 'Could not read Computer Use setup. Check the backend connection and try again.'
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
        aria-controls="computer-use-setup-details"
        onClick={toggle}
      >
        {loading && <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />}
        {loading
          ? 'Checking…'
          : !runtime
            ? 'Check Computer Use setup'
            : open
              ? 'Hide Computer Use setup'
              : 'Show Computer Use setup'}
        {runtime && !loading && (
          <ChevronDown
            className={`size-3.5 shrink-0 transition-transform ${open ? 'rotate-180' : 'rotate-0'}`}
            aria-hidden="true"
          />
        )}
      </Button>
      {expanded && runtime && (
        <div id="computer-use-setup-details" className="space-y-2">
          <ComputerUseRuntimeDetails runtime={runtime} />
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
