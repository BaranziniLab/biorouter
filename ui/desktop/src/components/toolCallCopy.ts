/**
 * The words on the transcript's tool rows and on the cards that ask the person
 * to act (approval, elicitation, secret request, artifact card). One module so
 * the components and their tests read the same strings (Crew's `copy.ts`
 * pattern). Sentence case, "chat" not "conversation", no dashes.
 */

/** A tool row's state, carried by its verb. */
export type ToolRowState = 'running' | 'done' | 'failed' | 'stopped';

/**
 * The summaries `summarizeToolCall` writes start with a gerund ("Reading
 * package.json", "Running ls"). A finished row reads in the past tense and a
 * failed one says what it could not do, so each known gerund carries its past
 * tense and its base form. Codex: "Ran cat replay", "Read foo.txt".
 */
const VERB_FORMS: Record<string, readonly [past: string, base: string]> = {
  Adding: ['Added', 'add'],
  Applying: ['Applied', 'apply'],
  Attaching: ['Attached', 'attach'],
  Breaking: ['Broke', 'break'],
  Browsing: ['Browsed', 'browse'],
  Capturing: ['Captured', 'capture'],
  Checking: ['Checked', 'check'],
  Closing: ['Closed', 'close'],
  Creating: ['Created', 'create'],
  Delegating: ['Delegated', 'delegate'],
  Detaching: ['Detached', 'detach'],
  Downloading: ['Downloaded', 'download'],
  Editing: ['Edited', 'edit'],
  Executing: ['Executed', 'execute'],
  Fetching: ['Fetched', 'fetch'],
  Generating: ['Generated', 'generate'],
  Inspecting: ['Inspected', 'inspect'],
  Installing: ['Installed', 'install'],
  Listing: ['Listed', 'list'],
  Loading: ['Loaded', 'load'],
  Managing: ['Managed', 'manage'],
  Marking: ['Marked', 'mark'],
  Opening: ['Opened', 'open'],
  Posting: ['Posted', 'post'],
  Previewing: ['Previewed', 'preview'],
  Reading: ['Read', 'read'],
  Removing: ['Removed', 'remove'],
  Renaming: ['Renamed', 'rename'],
  Replacing: ['Replaced', 'replace'],
  Returning: ['Returned', 'return'],
  Running: ['Ran', 'run'],
  Searching: ['Searched', 'search'],
  Selecting: ['Selected', 'select'],
  Sending: ['Sent', 'send'],
  Sharing: ['Shared', 'share'],
  Starting: ['Started', 'start'],
  Stopping: ['Stopped', 'stop'],
  Unloading: ['Unloaded', 'unload'],
  Updating: ['Updated', 'update'],
  Using: ['Used', 'use'],
  Writing: ['Wrote', 'write'],
};

/**
 * The visible row label for a summary in a given state.
 *
 * - running: the summary itself ("Reading package.json"); it already says the
 *   work is under way.
 * - done: past tense ("Read package.json").
 * - failed: "Failed to read package.json".
 * - stopped: "Stopped reading package.json".
 *
 * A summary that does not open with a known gerund (a tool's display name, a
 * plan step's own words such as "Read the manifest") cannot be conjugated, and
 * a prefix verb would read "Ran Read the manifest". So it stands alone while
 * running (the pulse says so) and once done, and failure or a stop is named
 * before it: "Failed: X", "Stopped: X".
 */
export function toolRowLabel(summary: string, state: ToolRowState): string {
  const match = /^([A-Z][a-z]+)\b(.*)$/s.exec(summary);
  const forms = match ? VERB_FORMS[match[1]] : undefined;
  if (match && forms) {
    const [gerund, rest] = [match[1], match[2]];
    const [past, base] = forms;
    switch (state) {
      case 'running':
        return summary;
      case 'done':
        return `${past}${rest}`;
      case 'failed':
        return `Failed to ${base}${rest}`;
      case 'stopped':
        return `Stopped ${gerund.toLowerCase()}${rest}`;
    }
  }
  switch (state) {
    case 'running':
    case 'done':
      return summary;
    case 'failed':
      return `Failed: ${summary}`;
    case 'stopped':
      return `Stopped: ${summary}`;
  }
}

export const TOOL_ROW_COPY = {
  preparing: 'Preparing',
  statusLabel: (status: string) => `Tool status: ${status}`,

  // Well sections
  input: 'Input',
  steps: 'Steps',
  code: 'Code',
  output: 'Output',
  logs: 'Logs',
  liveLogs: 'Live logs',
  progress: 'Progress',
  error: 'Tool call failed',
  executedCalls: (count: number) => `Executed calls (${count})`,
  recordedCalls: (recorded: number, total: number) =>
    `Recorded calls (${recorded} of ${total} executed)`,
  notRecorded: (count: number) => `${count} not recorded`,
  notRecordedHelp: (count: number) =>
    count === 1
      ? 'One executed call was not recorded, so its details are unavailable.'
      : `${count} executed calls were not recorded, so their details are unavailable.`,
  noArguments: 'No arguments recorded.',
  stepTool: (tool: string) => `Tool: ${tool}`,
  stepUses: (dependencies: string) => ` (uses ${dependencies})`,
  imageAlt: 'Tool result',
  progressLabel: 'Tool progress',

  // A mirrored call that ran in the coding agent's own sandbox.
  notGated: 'not gated by Biorouter',
  notGatedName: 'Not gated by Biorouter',
  notGatedHelp:
    "This tool ran inside the coding agent's own sandbox. Biorouter's inspectors, " +
    'permission mode, .biorouterignore and privacy gates did not apply to it.',

  // The render boundary's error row.
  renderFailed: 'Tool details unavailable',
  renderFailedMessage: 'This tool returned an unexpected response. The chat can continue.',
  details: 'Details',

  // Tool content previews
  showMore: 'Show more',
  showLess: 'Show less',
} as const;

export const APPROVAL_COPY = {
  question: 'Run',
  allowOnce: 'Allow once',
  alwaysAllow: 'Always allow',
  deny: 'Deny',
  change: 'Change',
  canceled: 'Approval canceled',
  unavailable: 'Approval is no longer available',
  resolved: (tool: string, outcome: string) => `${tool} is ${outcome}`,
  outcome: {
    alwaysAllow: 'always allowed',
    allowOnce: 'allowed once',
    deny: 'denied',
    alreadyResolved: 'already answered',
    unknown: 'no longer available',
  },
  confirmFailed: 'Could not confirm your decision. Try again.',
  // A refusal, so it stays visible (principle 2).
  browserCannotApprove:
    'This page is served to a browser, which has no way to prove a decision came from you ' +
    'rather than from the model. Answer this request in the Biorouter desktop app.',
} as const;

export const PREVIEW_COPY = {
  command: 'Command',
  arguments: 'Arguments',
  newFile: (lines: number) => `New file · ${lines} ${lines === 1 ? 'line' : 'lines'}`,
  truncated: 'Truncated',
  truncatedHelp: 'The full call is larger than this preview.',
  risk: {
    low: 'Read-only',
    medium: 'Modifies data',
    high: 'Destructive',
    unknown: 'Unverified',
  },
} as const;

export const ELICITATION_COPY = {
  fallbackQuestion: 'Biorouter needs some information from you.',
  submit: 'Submit',
  submitting: 'Submitting…',
  sent: 'Information sent',
  canceled: 'Information request canceled',
} as const;

export const SECRET_COPY = {
  fallbackPrompt: 'Biorouter needs some credentials.',
  reassurance: 'Saved on this machine. The model never sees it.',
  reassuranceHelp:
    "These values go straight to this machine's credential store. They are not added to " +
    'the chat, and the model never sees them.',
  optional: '(optional)',
  stillNeeded: (keys: string[]) => `Still needed: ${keys.join(', ')}`,
  cancel: 'Cancel',
  save: 'Save and continue',
  saving: 'Saving…',
  storeFailed: 'Biorouter could not store these values.',
  canceled: 'Credential setup canceled. Nothing was installed.',
  gone: (extension: string | null) =>
    `This request is no longer waiting for an answer. Ask again to configure ${
      extension ?? 'the extension'
    }.`,
  configured: (extension: string | null, keys: string[]) =>
    `Credentials configured${extension ? ` for ${extension}` : ''}${
      keys.length > 0 ? `: ${keys.join(', ')}` : ''
    }`,
} as const;

export const ARTIFACT_CARD_COPY = {
  kind: {
    figure: 'Figure',
    report: 'Report',
    app: 'App',
    page: 'Page',
    resource: 'Resource',
  },
  fallbackTitle: 'Artifact',
  openLabel: (title: string, external: boolean) =>
    `Open ${title} ${external ? 'in the default browser' : 'in the artifact viewer'}`,
  noPreview: (title: string) => `No browser-safe preview is available for ${title}.`,
} as const;
