/**
 * The words of first run (codex simplicity redesign, R-34): the page's band and the setup panels a
 * provider row opens in place. Tests import these instead of retyping them.
 *
 * Help strings are InfoTip text: full sentences, at most two, about 160 characters. Labels and
 * buttons take no trailing period and no arrow. Toasts are sentence case with no exclamation mark.
 *
 * ⚠ Privacy copy is not here and never moves here: the tier notes (`providerOrdering.ts`), the
 * host-managed panel (`privacy/hostManagedModelCopy.ts`) and the non-private disclosure are their
 * own files, frozen by ruling.
 */

/** The first-run page's band (`ProviderGuard`). */
export const firstRunCopy = {
  /** The desktop's title: the same task, and the same words, as the `#/welcome` page. */
  title: 'Choose a provider',
  /** A browser served by `biorouter serve`, where the choice is the host's. */
  hostTitle: 'Get started',
  /** Into the app with no provider bound; the composer says why it cannot send. */
  skip: 'Explore Biorouter first',
  openCrew: 'Open Crew',
} as const;

/** Llama Server's panel (`LlamaServerInlineCard`), the row first run opens by default. */
export const llamaCopy = {
  model: 'Model',
  /** The disclosure holding what the facts line leaves out: the ids, the paths, the fit. */
  details: 'Details',
  /** Only news is a status: a running server, or one that cannot start. */
  binaryMissing: 'llama-server binary not found',
  running: 'Running · ready',
  warmUpNeeded: 'Running · warm-up needed',
  checking: 'Checking Llama Server…',
  /** The one action, on the control ladder's 32px rung, with the size it will download. */
  start: 'Start',
  downloadAndStart: (size?: string) =>
    size ? `Download and start (${size})` : 'Download and start',
  resumeAndStart: (size: string) => `Resume and start (${size})`,
  warmUp: 'Warm up model',
  settingUp: 'Setting up…',
  use: 'Use Llama Server',
  connecting: 'Connecting…',
  ready: 'Local model ready',
  readyDetail: (model: string) => `Llama Server is running ${model} on your computer.`,
  /** The Details rows the Settings info dialog does not have; the rest share its labels. */
  download: 'Download',
} as const;

/** The institutional gateway form a Versa row opens in place (`InstitutionalSetupCard`). */
export const institutionalCopy = {
  apiKey: 'API key',
  accessKeyId: 'Access key ID',
  secretAccessKey: 'Secret access key',
  advanced: 'Advanced',
  azureSummary: 'Endpoint, API version',
  bedrockSummary: 'Endpoint, region',
  endpoint: 'Endpoint',
  apiVersion: 'API version',
  region: 'Region',
  /** A field's InfoTip: the config key it writes is machine text, so it is the tip, not the label. */
  savedAs: (key: string) => `Saved as ${key}.`,
  connect: 'Connect',
  connecting: 'Connecting…',
  /** The daemon's refusal is the sentence that says what is wrong, so it is shown as it came. */
  refused: (said: string) => `Could not connect: ${said}`,
  refusedPlain: 'Could not connect with these credentials.',
} as const;
