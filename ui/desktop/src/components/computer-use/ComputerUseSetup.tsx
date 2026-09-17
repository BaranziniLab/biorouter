import { useState } from 'react';
import { Button } from '../ui/button';
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
      {/darwin/.test(target) ? (
        <p>
          Allow the Biorouter Computer Use helper in System Settings → Privacy &amp; Security →
          Accessibility and Screen Recording, then check again.
        </p>
      ) : /windows/.test(target) ? (
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
  const check = async () => {
    setLoading(true);
    setError('');
    try {
      setRuntime(await computerUseSetup());
    } catch (failure) {
      setError(
        failure instanceof Error
          ? failure.message
          : 'Could not read Computer Use setup. Check the backend connection and try again.'
      );
    } finally {
      setLoading(false);
    }
  };
  return (
    <div className="mt-2 space-y-2">
      <Button size="sm" variant="outline" disabled={loading} onClick={() => void check()}>
        {loading ? 'Checking…' : 'Check Computer Use setup'}
      </Button>
      {runtime && <ComputerUseRuntimeDetails runtime={runtime} />}
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
