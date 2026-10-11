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
  launch: 'Launch',
  /** A row's own name, so repeated buttons are not all "Configure". */
  configureRow: (name: string) => `Configure ${name}`,
  launchRow: (name: string) => `Launch ${name}`,
  emptyTab: 'Nothing here yet.',
  /** Shown under the Institutional tab, once. */
  otherInstitutions: "Don't see your institution?",
  otherInstitutionsHelp:
    'A gateway appears here once Biorouter recognizes its endpoint as private. Anything else can be added under Public with Add custom provider, and is treated as public.',
} as const;

export const customProviderCopy = {
  add: 'Add provider',
  setUp: (name: string) => `Set up ${name}`,
  edit: (name: string) => `Edit ${name}`,
  configure: (name: string) => `Configure ${name}`,
  providerType: 'Provider type',
  displayName: 'Display name',
  apiUrl: 'API URL',
  apiKey: 'API key',
  models: 'Available models',
  localModel: 'This is a local model (no auth required)',
  streaming: 'Provider supports streaming responses',
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
  cancel: 'Cancel',
  remove: 'Remove',
  delete: 'Delete',
  ok: 'OK',
  save: 'Save',
  enable: 'Enable',
  checking: 'Checking…',
} as const;
