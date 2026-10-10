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
  /** First run only: where the person is and what they can do next. */
  onboardingLead: 'Pick where your models run. You can switch providers any time in settings.',
  loading: 'Loading providers…',
} as const;

export const catalogCopy = {
  addCustom: 'Add custom provider',
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
  deleteTitle: (name: string) => `Delete configuration for ${name}`,
  cancel: 'Cancel',
  remove: 'Remove',
  delete: 'Delete',
  ok: 'OK',
  save: 'Save',
  enable: 'Enable',
  checking: 'Checking…',
} as const;
