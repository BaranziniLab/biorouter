/**
 * The words of Biorouter Copilot's setup check (Settings > Chat > Capabilities, and the readiness
 * lines the in-chat banner shares). Tests import these instead of retyping them.
 *
 * The trigger is a row-trailing button, so its label is a verb and nothing else: the row already
 * says "Biorouter Copilot". Result lines carry no dashes as punctuation.
 */
export const copilotSetupCopy = {
  /** The row button, by state. */
  check: 'Check setup',
  checking: 'Checking…',
  hide: 'Hide setup',
  show: 'Show setup',
  /** The re-check inside the result panel. */
  checkAgain: 'Check again',
  readFailed: 'Could not read Biorouter Copilot setup. Check the backend connection and try again.',
  /**
   * Who holds the macOS grants (AD-04): the Biorouter Copilot bundle, never Biorouter itself, so a
   * command Biorouter's shell runs can never borrow them.
   */
  macGrantOwner:
    'Allow Biorouter Copilot under Accessibility, and under Screen Recording for screenshots. Biorouter itself needs neither.',
  macGrantHelp:
    'If macOS asks you to quit and reopen it, do so, then use Check again. Checking permissions does not grant them.',
  /** The one-press request for the system's prompts (macOS, local backend). */
  request: 'Request access',
  requesting: 'Requesting…',
  requestHelp:
    'macOS shows its own prompts for Biorouter Copilot. Allow Accessibility and Screen Recording. It may also ask about System Events, which Biorouter Copilot does not use, so you can refuse that one.',
  requestFailed: 'Could not ask macOS for access. Open the settings below instead.',
  /** What the native runtime reports, in words. Keys are the daemon's `runtime.status`. */
  runtimeStates: {
    ready: 'Runtime ready',
    missing_runtime: 'Bundled runtime missing',
    incompatible_runtime: 'Runtime incompatible',
    desktop_unavailable: 'Desktop unavailable',
    probe_pending: 'Runtime found, setup not checked',
    os_permission_required: 'OS permission required',
    unsupported_environment: 'Unsupported desktop environment',
    missing_dependency: 'System dependency missing',
    probe_failed: 'Could not check the native runtime',
  } as Record<string, string>,
  /** The outcome of a re-check, by verdict (never colour alone: the words carry it). */
  verdicts: {
    ready: 'All OS permissions are allowed.',
    blocked: 'A required OS permission is missing. See the detail above.',
    unavailable: 'The native runtime is unavailable. See the repair steps above.',
    unverified: 'The backend could not confirm OS permissions. See the detail above.',
  },
} as const;
