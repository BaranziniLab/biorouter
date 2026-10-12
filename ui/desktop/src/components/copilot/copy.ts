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
