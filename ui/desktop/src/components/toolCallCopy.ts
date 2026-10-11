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
  Analyzing: ['Analyzed', 'analyze'],
  Applying: ['Applied', 'apply'],
  Attaching: ['Attached', 'attach'],
  Authenticating: ['Authenticated', 'authenticate'],
  Breaking: ['Broke', 'break'],
  Browsing: ['Browsed', 'browse'],
  Building: ['Built', 'build'],
  Calling: ['Called', 'call'],
  Canceling: ['Canceled', 'cancel'],
  Capturing: ['Captured', 'capture'],
  Checking: ['Checked', 'check'],
  Clearing: ['Cleared', 'clear'],
  Closing: ['Closed', 'close'],
  Configuring: ['Configured', 'configure'],
  Converting: ['Converted', 'convert'],
  Copying: ['Copied', 'copy'],
  Creating: ['Created', 'create'],
  Delegating: ['Delegated', 'delegate'],
  Describing: ['Described', 'describe'],
  Detaching: ['Detached', 'detach'],
  Downloading: ['Downloaded', 'download'],
  Editing: ['Edited', 'edit'],
  Executing: ['Executed', 'execute'],
  Exporting: ['Exported', 'export'],
  Fetching: ['Fetched', 'fetch'],
  Finding: ['Found', 'find'],
  Generating: ['Generated', 'generate'],
  Getting: ['Got', 'get'],
  Importing: ['Imported', 'import'],
  Ingesting: ['Ingested', 'ingest'],
  Inspecting: ['Inspected', 'inspect'],
  Installing: ['Installed', 'install'],
  Linting: ['Linted', 'lint'],
  Listing: ['Listed', 'list'],
  Loading: ['Loaded', 'load'],
  Looking: ['Looked', 'look'],
  Managing: ['Managed', 'manage'],
  Marking: ['Marked', 'mark'],
  Merging: ['Merged', 'merge'],
  Moving: ['Moved', 'move'],
  Opening: ['Opened', 'open'],
  Parsing: ['Parsed', 'parse'],
  Posting: ['Posted', 'post'],
  Previewing: ['Previewed', 'preview'],
  Querying: ['Queried', 'query'],
  Reading: ['Read', 'read'],
  Recalling: ['Recalled', 'recall'],
  Removing: ['Removed', 'remove'],
  Renaming: ['Renamed', 'rename'],
  Rendering: ['Rendered', 'render'],
  Replacing: ['Replaced', 'replace'],
  Retrieving: ['Retrieved', 'retrieve'],
  Returning: ['Returned', 'return'],
  Running: ['Ran', 'run'],
  Saving: ['Saved', 'save'],
  Scheduling: ['Scheduled', 'schedule'],
  Searching: ['Searched', 'search'],
  Selecting: ['Selected', 'select'],
  Sending: ['Sent', 'send'],
  Setting: ['Set', 'set'],
  Sharing: ['Shared', 'share'],
  Showing: ['Showed', 'show'],
  Starting: ['Started', 'start'],
  Stopping: ['Stopped', 'stop'],
  Storing: ['Stored', 'store'],
  Summarizing: ['Summarized', 'summarize'],
  Syncing: ['Synced', 'sync'],
  Testing: ['Tested', 'test'],
  Unloading: ['Unloaded', 'unload'],
  Updating: ['Updated', 'update'],
  Uploading: ['Uploaded', 'upload'],
  Using: ['Used', 'use'],
  Validating: ['Validated', 'validate'],
  Viewing: ['Viewed', 'view'],
  Waiting: ['Waited', 'wait'],
  Watching: ['Watched', 'watch'],
  Writing: ['Wrote', 'write'],
};

/** Words a tool name spells in capitals inside a sentence-case label. */
const NAME_ACRONYMS = new Set([
  'api',
  'baam',
  'cdw',
  'csv',
  'ehr',
  'html',
  'http',
  'id',
  'json',
  'kb',
  'llm',
  'mcp',
  'okf',
  'omop',
  'pdf',
  'sql',
  'ucsf',
  'ui',
  'url',
]);

/** Product names keep their capitals (copy rules), whatever the identifier spelled. */
const PRODUCT_NAMES: ReadonlyArray<readonly [RegExp, string]> = [
  [/\bagent drafter\b/g, 'Agent Drafter'],
  [/\bauto ?visualiser\b/g, 'Auto Visualiser'],
  [/\bbiorouter\b/g, 'Biorouter'],
  [/\bcodex\b/g, 'Codex'],
  [/\bcrew\b/g, 'Crew'],
  [/\bgithub\b/g, 'GitHub'],
];

/**
 * A tool identifier as lowercase words: `ingest_conversation`, `kbSearch`,
 * `files/read` and `web-scrape` all split the same way.
 */
export function toolNameWords(identifier: string): string[] {
  return identifier
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean)
    .map((word) => word.toLowerCase());
}

/**
 * A tool identifier as words that read inside a sentence: lowercase, with
 * acronyms and product names in capitals ("text editor", "UCSF OMOP query",
 * "Agent Drafter build app"). The transcript never shows Title Case.
 */
export function toolNamePhrase(identifier: string | readonly string[]): string {
  const words = typeof identifier === 'string' ? toolNameWords(identifier) : identifier;
  return PRODUCT_NAMES.reduce(
    (text, [pattern, name]) => text.replace(pattern, name),
    words.map((word) => (NAME_ACRONYMS.has(word) ? word.toUpperCase() : word)).join(' ')
  );
}

/** The first letter in capitals, the rest as written. */
export function sentenceStart(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/**
 * Words written mid-sentence: a step a model described in its own words opens
 * in lowercase ("Read the manifest" → "read the manifest") so it can follow a
 * verb. Only a plainly capitalized first word moves: "KBs", "SQL" and a
 * product name ("Biorouter", "Agent Drafter") keep their capitals.
 */
export function inSentence(text: string): string {
  if (!/^[A-Z][a-z]+(?![A-Za-z0-9])/.test(text)) return text;
  const probe = text.toLowerCase();
  const named = PRODUCT_NAMES.reduce(
    (phrase, [pattern, name]) => phrase.replace(pattern, name),
    probe
  );
  return named.charAt(0) === probe.charAt(0) ? text.charAt(0).toLowerCase() + text.slice(1) : text;
}

/**
 * The name a server gives its own tools when it prefixes them with it:
 * `medcp__MedCP-list_clinical_tables` → "MedCP". That prefix is a product
 * name, so a row keeps its spelling ("Listed MedCP clinical tables"); split
 * as an identifier it read "med cp" (V2-R2-5). Null for any other tool.
 */
export function toolServerName(identifier: string): string | null {
  const parts = identifier.split('__');
  if (parts.length < 2) return null;
  const match = /^([A-Za-z][A-Za-z0-9]*)-(?=[A-Za-z0-9])/.exec(parts[parts.length - 1]);
  if (!match) return null;
  const key = (text: string) => text.toLowerCase().replace(/[^a-z0-9]/g, '');
  return key(match[1]) === key(parts[0]) ? match[1] : null;
}

/**
 * A full tool name (`extension__tool`) as the lowercase words of the tool's
 * own name, with a server prefix ({@link toolServerName}) kept as one word.
 */
export function toolCallNameWords(identifier: string): string[] {
  const tool = identifier.split('__').pop() ?? identifier;
  const server = toolServerName(identifier);
  return server
    ? [server.toLowerCase(), ...toolNameWords(tool.slice(server.length + 1))]
    : toolNameWords(tool);
}

/** `text` with the server's name ({@link toolServerName}) back in its own spelling. */
export function spellServerName(text: string, server: string | null): string {
  if (!server) return text;
  // A server name is letters and digits only, so it needs no escaping.
  const pattern = new RegExp(`(^|[^A-Za-z0-9])${server}(?![A-Za-z0-9])`, 'i');
  return text.replace(pattern, (_, before: string) => `${before}${server}`);
}

/**
 * A full tool name as words for the middle of a sentence ("Run text editor?",
 * "Preparing MedCP list clinical tables"): {@link toolNamePhrase} with the
 * server's own spelling kept.
 */
export function toolCallPhrase(identifier: string): string {
  return spellServerName(toolNamePhrase(toolCallNameWords(identifier)), toolServerName(identifier));
}

/** A tool identifier as a sentence-case name: `developer__text_editor` → "Text editor". */
export function toolDisplayName(identifier: string): string {
  return sentenceStart(toolCallPhrase(identifier));
}

/** Base verb → gerund, read off {@link VERB_FORMS} so every gerund it writes conjugates. */
const NAME_VERBS: Record<string, string> = {
  ...Object.fromEntries(Object.entries(VERB_FORMS).map(([gerund, [, base]]) => [base, gerund])),
  lookup: 'Looking up',
};

/**
 * Verbs that name the action wherever they sit in a tool's name
 * (`cohort_lookup`, `workspace_watch`). Every other verb counts only as the
 * first word, because words such as `test` or `set` are nouns at the end of a
 * name (`smoke_test`, `data_set`).
 */
const VERBS_ANYWHERE = new Set([
  'authenticate',
  'configure',
  'ingest',
  'lookup',
  'render',
  'retrieve',
  'summarize',
  'validate',
  'watch',
]);

/**
 * What a tool does, as a gerund phrase built from its name, so the row's verb
 * can carry the state: `ingest_conversation` → "Ingesting conversation" (done:
 * "Ingested conversation"), `cohort_lookup` → "Looking up cohort". A name
 * with no verb in it runs: `spreadsheet_summary` → "Running spreadsheet
 * summary", "Ran spreadsheet summary" once done.
 */
export function toolActionPhrase(words: readonly string[]): string {
  const index =
    words.length > 0 && NAME_VERBS[words[0]] !== undefined
      ? 0
      : words.findIndex((word) => VERBS_ANYWHERE.has(word));
  if (index === -1) return words.length > 0 ? `Running ${toolNamePhrase(words)}` : 'Running a tool';
  const rest = words.filter((_, position) => position !== index);
  return rest.length > 0
    ? `${NAME_VERBS[words[index]]} ${toolNamePhrase(rest)}`
    : NAME_VERBS[words[index]];
}

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
  noOutput: 'No output',
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

/**
 * What an Auto Visualiser figure kind is called on a row: "Rendering volcano
 * plot", not "Render figure · volcano". Keys are the kind slugs `render_figure`
 * takes and the legacy per-kind tool names without their `render_`/`show_`.
 */
export const FIGURE_NAMES: Readonly<Record<string, string>> = {
  area: 'area chart',
  boxplot: 'box plot',
  bubble: 'bubble chart',
  calendar_heatmap: 'calendar heatmap',
  chart: 'chart',
  choropleth: 'choropleth map',
  chord: 'chord diagram',
  class_diagram: 'class diagram',
  dashboard: 'report',
  dendrogram: 'dendrogram',
  donut: 'donut chart',
  er_diagram: 'ER diagram',
  flowchart: 'flowchart',
  forest: 'forest plot',
  gantt: 'Gantt chart',
  gauge: 'gauge',
  heatmap: 'heatmap',
  histogram: 'histogram',
  kaplan_meier: 'Kaplan-Meier curve',
  manhattan: 'Manhattan plot',
  map: 'map',
  mermaid: 'diagram',
  mindmap: 'mind map',
  network: 'network',
  radar: 'radar chart',
  report: 'report',
  sankey: 'Sankey diagram',
  sequence: 'sequence diagram',
  state_diagram: 'state diagram',
  sunburst: 'sunburst',
  timeline: 'timeline',
  treemap: 'treemap',
  volcano: 'volcano plot',
  wordcloud: 'word cloud',
};

export const FIGURE_COPY = {
  render: (figure: string) => `Rendering ${figure}`,
  renderAny: 'Rendering a figure',
  describe: (figure: string) => `Checking the ${figure} format`,
  describeAll: 'Listing figure kinds',
} as const;

/** One row for a run of reads, lists and searches (D18, Codex's "Explored"). */
export const EXPLORED_COPY = {
  running: 'Exploring',
  done: 'Explored',
  /** "3 files, 1 folder, 2 searches": only the kinds the run holds, in that order. */
  counts: ({ read, list, search }: { read: number; list: number; search: number }) =>
    [
      read > 0 ? `${read} ${read === 1 ? 'file' : 'files'}` : null,
      list > 0 ? `${list} ${list === 1 ? 'folder' : 'folders'}` : null,
      search > 0 ? `${search} ${search === 1 ? 'search' : 'searches'}` : null,
    ]
      .filter((part): part is string => part !== null)
      .join(', '),
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
  reassuranceName: 'Credential storage',
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
