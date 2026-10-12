/**
 * The words of the provider catalog and its dialogs (codex simplicity redesign, spec §3.13).
 *
 * Tests import these instead of retyping them. Help strings are InfoTip text: full sentences, at
 * most two, about 160 characters. Labels take no trailing period.
 *
 * ⚠ The tier notes in `providerOrdering.ts` are PRIVACY information (§14.5) and stay visible, one
 * muted line per tab. Nothing in this file moves them into a tip.
 */

export const providerPageCopy = {
  /** The heading outside onboarding. */
  title: 'Provider configuration',
  onboardingTitle: 'Choose a provider',
  /**
   * First run's help, ahead of where keys are kept, in the title's InfoTip. It used to be a
   * visible line under the title that restated it (V4-29): the title says what to do, and the one
   * thing worth adding is that the choice is not final.
   */
  onboardingHelp: 'You can switch providers any time in Settings.',
  loading: 'Loading providers…',
} as const;

export const catalogCopy = {
  /** The tab list's name: Local, Institutional, Public. */
  tabsLabel: 'Provider groups',
  addCustom: 'Add custom provider',
  /** The model dialog's title after a provider was just set up. */
  chooseModel: 'Choose model',
  configured: 'Configured',
  configure: 'Configure',
  /**
   * First run's action on a configured provider that does not open in place (an API row, Versa API
   * Azure). The panels of the rows that do open say "Use Claude Code", "Use Codex", "Use Ollama",
   * so this is the same verb (vision FR-R2-4: it read "Launch", a second word for one choice on
   * the next tab). Settings never shows it: a Settings row's action is Configure.
   */
  use: 'Use',
  /** A row's own name, so repeated buttons are not all "Configure". */
  configureRow: (name: string) => `Configure ${name}`,
  useRow: (name: string) => `Use ${name}`,
  emptyTab: 'Nothing here yet.',
  /** Shown under the Institutional tab, once. */
  otherInstitutions: "Don't see your institution?",
  otherInstitutionsHelp:
    'A gateway appears here once Biorouter recognizes its endpoint as private. Anything else can be added under Public with Add custom provider, and is treated as public.',
} as const;

/**
 * A row's second line where the daemon's description is not a line (vision FR-R3-4): the two Versa
 * rows were two clauses each ("API Key only; endpoint and deployment are pre-configured", with a
 * "+"), GitHub Copilot's was a terminal instruction its own Configure dialog contradicts by asking
 * for a token, and Xiaomi MiMo's named the variable XIAOMI_MIMO_HOST. Keyed by provider id; the
 * daemon's sentence stays the row's accessible description (`ProviderCard`'s `summary`), the way
 * the coding agents' rows already work (`agentCopy.rowSummary`).
 *
 * The Versa lines name no institution: they sit under the institution's own section label, so
 * "UCSF · GPT models · API key" said UCSF three times in a row (vision FR-R4-5, principle 2).
 */
export const providerRowSummary: Readonly<Record<string, string>> = {
  versa_azure: 'GPT models · API key',
  versa_bedrock: 'Claude models · access key and secret',
  github_copilot: 'Your GitHub Copilot subscription · token',
  xiaomi_mimo: 'Xiaomi MiMo models through an OpenAI-compatible API',
};

/**
 * A daemon description as one row line, in one face (vision FR-R3-4, principle 3): the command a
 * description wraps in backticks is drawn as plain text, as the line's tooltip already drew it, and
 * a one-sentence line drops its closing period, as every label and one-line description does (6 of
 * 22 public lines ended with one and 16 did not). A description of several sentences keeps its
 * punctuation, since dropping only the last period would leave the others standing.
 */
export function providerRowLine(description: string): string {
  const text = description.replace(/`/g, '').trim();
  const oneSentence = !/[.!?]\s+\S/.test(text);
  return oneSentence && text.endsWith('.') && !text.endsWith('..') ? text.slice(0, -1) : text;
}

export const customProviderCopy = {
  add: 'Add provider',
  setUp: (name: string) => `Set up ${name}`,
  edit: (name: string) => `Edit ${name}`,
  configure: (name: string) => `Configure ${name}`,
  providerType: 'Provider type',
  providerTypeHelp:
    'The API your provider speaks. Most hosted gateways and local servers speak the OpenAI one.',
  displayName: 'Display name',
  apiUrl: 'API URL',
  apiUrlHelp: 'The address Biorouter sends requests to, usually ending in /v1.',
  apiKey: 'API key',
  models: 'Available models',
  modelsHelp: 'The model names this provider serves, separated by commas.',
  localModel: 'This is a local model',
  localModelHelp: 'A local server needs no API key, so the key field is turned off.',
  streaming: 'Provider supports streaming responses',
  streamingHelp:
    'Replies appear as they are written. Turn this off for a server that only returns whole replies.',
  cancel: 'Cancel',
  create: 'Create provider',
  update: 'Update provider',
  save: 'Save',
} as const;

export const providerSetupCopy = {
  configure: (name: string) => `Configure ${name}`,
  /** A coding agent's dialog opens as its setup, since the form is a command path, not a key. */
  setUp: (name: string) => `Set up ${name}`,
  deleteTitle: (name: string) => `Delete configuration for ${name}`,
  /** What a coding agent's form is, said once, because it has no key field to say it. */
  agentIntro: (name: string) =>
    `Use your installed ${name} command-line app and subscription sign-in. No API key is needed here.`,
  /** A provider that declares no secret: the one sentence that says why there is no key field. */
  noKey: (name: string) => `${name} needs no API key. Adjust how Biorouter reaches it.`,
  /** The delete states are consequences, so they stay visible at the moment of decision. */
  inUse: 'This provider is in use. Switch to a different model first.',
  deleteConsequence: 'This will permanently delete the current provider configuration.',
  needsAttention: 'Configuration needs attention',
  agentCheckFailed:
    'The configuration check did not complete. Follow the setup steps below, then retry.',
  checkFailed: 'There was an error checking this provider configuration.',
  tryAgain: 'Check your configuration again to use this provider.',
  completeCliSetup: 'Complete CLI setup before choosing a model.',
  editCommandPath: 'Edit command path',
  goBack: 'Go back',
  loadingValues: 'Loading configuration values…',
  noParameters: 'No configuration parameters for this provider.',
  /**
   * The optional fields' disclosure: the first-run path's one disclosure word (Llama Server's and
   * Versa's say it too), with the hidden fields' labels as its summary (vision FR-R4-5).
   */
  advanced: 'Advanced',
  advancedSummary: (labels: readonly string[]) => labels.join(' · '),
  cancel: 'Cancel',
  remove: 'Remove',
  delete: 'Delete',
  ok: 'OK',
  save: 'Save',
  enable: 'Enable',
  checking: 'Checking…',
} as const;
