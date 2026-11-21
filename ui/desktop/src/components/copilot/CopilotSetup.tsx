import { useState } from 'react';
import { Check } from '../icons/app-icons';
import { Button } from '../ui/button';
import { InfoTip } from '../ui/info-tip';
import { Note } from '../ui/note';
import { Spinner } from '../ui/spinner';
import { PermissionCheckButton } from './PermissionCheckButton';
import { copilotRequestPermissions, copilotSetup, type CopilotRuntime } from './copilotApi';
import { copilotSetupCopy } from './copy';

/** The panel's DOM id: the row's button controls it. */
export const COPILOT_SETUP_PANEL_ID = 'copilot-setup-details';

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

/**
 * Which runtime answered, for the state line's InfoTip: the backend computer and the runtime's
 * version and target. Facts a person rarely needs, so they wait behind the glyph rather than take
 * two lines of the panel. Empty when the runtime reported none.
 */
export function runtimeFacts(runtime: CopilotRuntime): string {
  const facts: string[] = [];
  if (runtime.host) facts.push(`Backend computer: ${runtime.host}.`);
  if (runtime.runtime_version) {
    facts.push(
      `Runtime version: ${runtime.runtime_version}${runtime.target ? ` · ${runtime.target}` : ''}.`
    );
  }
  return facts.join(' ');
}

/**
 * What a runtime check found, in layers (owner, 2026-11-13: the answer up front, the detail on
 * hover or on click). In view: the state, an error that just happened, the OS permissions, the
 * exact copy to grant, the backend's own message, and one line of what to do next, with the
 * buttons that do it. Behind an InfoTip beside the line it explains: which runtime answered, and
 * the second sentence of each remedy.
 */
export function CopilotRuntimeDetails({
  runtime,
  onRuntime,
}: {
  runtime: CopilotRuntime;
  /** Takes the readiness read again after the person asked the system for access. */
  onRuntime?: (runtime: CopilotRuntime) => void;
}) {
  const [settingsError, setSettingsError] = useState('');
  const [requesting, setRequesting] = useState(false);
  const [requestError, setRequestError] = useState('');
  const localMac =
    window.electron?.platform === 'darwin' &&
    window.electron?.getConfig?.().BIOROUTER_LOCAL_BACKEND === true &&
    /darwin/.test(runtime.target ?? '') &&
    Boolean(window.electron?.openCopilotPermissionSettings);
  // AD-04: the grants belong to the Biorouter Copilot bundle, which macOS lists only after it has
  // asked. One press shows the system's own prompts, naming Biorouter Copilot, on this computer.
  const requestAccess = async () => {
    if (requesting) return;
    setRequesting(true);
    setRequestError('');
    try {
      const next = await copilotRequestPermissions();
      if (next.request_error) setRequestError(next.request_error);
      onRuntime?.(next);
    } catch (error) {
      setRequestError(error instanceof Error ? error.message : copilotSetupCopy.requestFailed);
    } finally {
      setRequesting(false);
    }
  };
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
  const grantedCopy = /darwin/.test(target)
    ? (runtime.app_bundle ?? runtime.executable)
    : undefined;
  const verdict = runtimeVerdict(runtime);
  const facts = runtimeFacts(runtime);
  return (
    <div className="space-y-1 text-supporting text-text-muted">
      {/* The state is the answer; everything under it is detail. It takes the default ink so the
          block is read from its first line, where the muted lines under it are the how and why.
          Which runtime answered is its InfoTip. */}
      <p className="text-text-default">
        {copilotSetupCopy.runtimeStates[runtime.status] ?? runtime.status}
        {facts && <InfoTip label="this runtime" help={facts} />}
      </p>
      {runtime.error && <p className="break-words">{runtime.error}</p>}
      <p>OS permissions: {permissionSummary || 'Not reported by this environment.'}</p>
      {/* ⚠ NAME THE BINARY. macOS can key a grant to a particular executable PATH, not
          only to the signing identity, so a machine holding more than one copy of the
          engine (an installed Biorouter, a packaged build, a source build) may need each
          copy granted separately, and an ad hoc signed development build's grant follows
          its code hash. Without this line the panel says "allow Biorouter Copilot" to
          someone who has already done exactly that, for a different copy, and nothing on
          screen tells them so. Measured 2026-09-23 with the previous engine: three copies
          on one machine, two reporting `screen_recording: true` and the source build
          reporting false, with an identical bundle id and the same Developer ID. */}
      {/* The path is data, read character by character against what System Settings lists, so
          it is set as code (design.md D-42), not as a folder's place. On macOS that is the
          Biorouter Copilot app, not the engine inside it; a development payload outside any app
          is granted by its own executable. Windows and Linux grant nothing per copy. */}
      {grantedCopy && verdict !== 'ready' && (
        <p className="break-all">
          Grant these to this exact copy: <code className="font-mono text-code">{grantedCopy}</code>
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
          The permission check failed; this does not mean access was denied.
          <InfoTip
            label="recovering from a failed check"
            help="Restart Biorouter on the backend computer and check again. If it still fails, repair the matching Biorouter package."
          />
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
            Biorouter Copilot has the access it needs on the backend computer. Nothing further to
            set up.
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
          reported above.
          <InfoTip
            label="Linux desktop permissions"
            help="Desktop portals may request additional permissions."
          />
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
                : copilotSetupCopy.macGrantOwner}
              <InfoTip label="allowing permissions on macOS" help={copilotSetupCopy.macGrantHelp} />
            </p>
            {localMac ? (
              <div className="flex flex-wrap items-center gap-2">
                {/* One press does it (owner, 2026-11-19): the system's own prompts appear, naming
                    Biorouter Copilot. Settings stay beside it for a grant that was refused. */}
                <Button
                  size="sm"
                  aria-busy={requesting}
                  aria-disabled={requesting || undefined}
                  className="aria-disabled:pointer-events-none aria-disabled:opacity-50"
                  onClick={() => void requestAccess()}
                >
                  {requesting && <Spinner />}
                  {requesting ? copilotSetupCopy.requesting : copilotSetupCopy.request}
                </Button>
                <InfoTip label="requesting access" help={copilotSetupCopy.requestHelp} />
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => void openSettings('accessibility')}
                >
                  Open Accessibility settings
                </Button>
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => void openSettings('screen_recording')}
                >
                  Open Screen Recording settings
                </Button>
              </div>
            ) : (
              <p>
                Make these changes on the backend computer.
                <InfoTip
                  label="a remote backend"
                  help="If it is remote, changing permissions on this device will not change the backend's access."
                />
              </p>
            )}
            {settingsError && <p role="alert">{settingsError}</p>}
            {requestError && <p role="alert">{requestError}</p>}
          </div>
        )}
      {runtime.development_override && (
        <p>
          Using an explicitly configured development runtime (BIOROUTER_COMPUTER_USE_DIR).
          <InfoTip
            label="the development runtime"
            help="Its operating-system permissions are separate from the installed app's."
          />
        </p>
      )}
    </div>
  );
}

/**
 * Everything the setup check remembers, shared by the row's button and the result panel that
 * follows the row. The two are separate components because they sit in different places: the
 * button is the row's trailing control, and the panel is a block after the row, so a long result
 * never swells the row it belongs to.
 */
export interface CopilotSetupState {
  runtime?: CopilotRuntime;
  error: string;
  loading: boolean;
  /** A check has run and answered. */
  checked: boolean;
  /** The panel is showing a result. */
  expanded: boolean;
  /** The first press checks; later presses show or hide what is already known. */
  toggle: () => void;
  /** Ask the backend again. */
  check: () => Promise<void>;
  /** Show a readiness read elsewhere, such as after the person asked the system for access. */
  adopt: (runtime: CopilotRuntime) => void;
}

export function useCopilotSetup(): CopilotSetupState {
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
      setError(failure instanceof Error ? failure.message : copilotSetupCopy.readFailed);
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

  const adopt = (next: CopilotRuntime) => {
    setRuntime(next);
    setChecked(true);
    setError('');
  };

  return {
    runtime,
    error,
    loading,
    checked,
    expanded: open && Boolean(runtime),
    toggle,
    check,
    adopt,
  };
}

/**
 * The row's trailing button: "Check setup" until a result exists, then "Hide setup" and
 * "Show setup". `describedBy` names the row it sits in, so the bare verb still says what it is for.
 */
export function CopilotSetupTrigger({
  setup,
  describedBy,
}: {
  setup: CopilotSetupState;
  describedBy?: string;
}) {
  const { runtime, loading, expanded, toggle } = setup;
  const label = loading
    ? copilotSetupCopy.checking
    : !runtime
      ? copilotSetupCopy.check
      : expanded
        ? copilotSetupCopy.hide
        : copilotSetupCopy.show;
  return (
    // ⚠ `aria-disabled`, not `disabled`, while a check runs. A `disabled` button that held keyboard
    // focus loses it (the browser moves focus to <body>), so pressing Enter on "Check setup" threw
    // a keyboard user back to the top of the page; measured in Chromium. The press is ignored in
    // JS, the spinner and `aria-busy` say it is working, and the dimming is the same 50% the
    // `disabled` look had.
    <Button
      type="button"
      size="sm"
      variant="secondary"
      aria-disabled={loading || undefined}
      aria-busy={loading}
      aria-expanded={runtime ? expanded : undefined}
      aria-controls={runtime ? COPILOT_SETUP_PANEL_ID : undefined}
      aria-describedby={describedBy}
      className="aria-disabled:pointer-events-none aria-disabled:opacity-50"
      onClick={() => {
        if (!loading) toggle();
      }}
    >
      {loading && <Spinner />}
      {label}
    </Button>
  );
}

/**
 * The result, as a block after the row: a `Note` that never clamps, because it carries buttons a
 * "Show more" fold would hide. A failed check is its own danger note, so the error is read as
 * soon as it lands. The block enters with the shared `.br-enter` (it appears because the person
 * pressed the button, so it may glide; the rows below it snap down, they do not slide).
 */
export function CopilotSetupPanel({ setup }: { setup: CopilotSetupState }) {
  const { runtime, error, loading, checked, expanded, check, adopt } = setup;
  if (!(expanded && runtime) && !error) return null;
  return (
    <>
      {expanded && runtime && (
        <Note id={COPILOT_SETUP_PANEL_ID} unclamped className="br-enter mx-3 mb-2 mt-1">
          <div className="space-y-2">
            <CopilotRuntimeDetails runtime={runtime} onRuntime={adopt} />
            <PermissionCheckButton
              label={copilotSetupCopy.checkAgain}
              checking={loading}
              checked={checked}
              verdict={runtimeVerdict(runtime)}
              onCheck={() => void check()}
            />
          </div>
        </Note>
      )}
      {error && (
        <Note tone="danger" role="alert" className="br-enter mx-3 mb-2 mt-1">
          {error}
        </Note>
      )}
    </>
  );
}
