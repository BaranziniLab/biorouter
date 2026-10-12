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
  /**
   * The disclosure holding what the facts line leaves out: the ids, the paths, the fit. It says
   * "Advanced", the one word every disclosure on the first-run path says (Versa's form and every
   * Configure dialog too), with a `·` summary beside it (vision FR-R4-5: it read "Details" here,
   * "Advanced" on Versa and "Show 4 options" in a dialog).
   */
  advanced: 'Advanced',
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
  /** Where the fallback download stands, after its spec in the "Fallback download" row. */
  fallbackDownloaded: 'Downloaded',
  fallbackPartial: 'Partial',
  fallbackIfNeeded: 'Only if needed',
} as const;

/** The institutional gateway form a Versa row opens in place (`InstitutionalSetupCard`). */
export const institutionalCopy = {
  apiKey: 'API key',
  accessKeyId: 'Access key ID',
  secretAccessKey: 'Secret access key',
  advanced: 'Advanced',
  /** What the closed disclosure holds: its fields' labels, joined with `·` as every summary is. */
  azureSummary: 'Endpoint · API version',
  bedrockSummary: 'Endpoint · Region',
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

/**
 * The coding agents' rows and panels (`codingAgentControls`). Each state says one instruction and
 * keeps its reason in an InfoTip beside it (vision FR-S-3); the command a person runs is a copy
 * field; errors and the pill stay visible.
 */
export const agentCopy = {
  /** The row's second line, in place of the daemon's long sentence. */
  rowSummary: (plan: string) => `Your ${plan} subscription · no API key`,
  notInstalled: 'Run this in a terminal, then choose Check again.',
  notInstalledTipLabel: 'installing',
  notInstalledTip: (key: string) =>
    `Installed somewhere unusual (nvm, volta, bun, asdf)? Set ${key} to its full path in Settings instead.`,
  signedOut: "Sign in with your subscription in a terminal, then choose I've signed in.",
  signedOutTipLabel: 'signing in',
  signedOutTip:
    'Biorouter does not perform the sign-in. The credential stays between you and the vendor and never passes through Biorouter.',
  /** Signed in with an API key: not a fault, so an instruction with two ways out. */
  apiKeyLead: 'Sign in with your subscription, or use the metered',
  apiKeyTail: 'provider under API providers.',
  apiKeyTipLabel: 'API key sign-in',
  apiKeyTip:
    'With an API key a run would bill per token, and this provider removes API credentials from the environment it starts, so it cannot run this way.',
  indeterminate: 'Biorouter could not tell whether this CLI is usable.',
  /** The panel's version line: "Version 2.1.283 · Max plan · ada@example.edu" (FR-R3-6). */
  version: 'Version',
  /** A CLI found on disk that printed no version. */
  installed: 'Installed',
  plan: (name: string) => `${name} plan`,
  checkAgain: 'Check again',
  signedIn: "I've signed in",
  checking: 'Checking…',
  openTerminal: 'Open a terminal here',
  hideTerminal: 'Hide terminal',
  use: (name: string) => `Use ${name}`,
  connecting: 'Connecting…',
  retryConfiguration: 'Retry configuration',
  checkingConfiguration: 'Checking configuration…',
  couldNotCheck: 'Could not check',
  tryAgain: 'Try again',
} as const;

/** The key detector at the top of Public's API providers (`CommercialSetupCard`). */
export const commercialCopy = {
  apiKey: 'API key',
  /**
   * The key field's InfoTip. It used to run the whole detectable list into the clause ("…, OpenAI,
   * z.ai, Xiaomi MiMo and Biorouter detects the provider.", vision FR-R3-4): the first three names
   * now stand for the rest, and the no-match error still names them all.
   */
  help: (supported: readonly string[]) => {
    const named = supported.slice(0, 3);
    const more = supported.length > named.length;
    const list =
      named.length <= 1
        ? (named[0] ?? 'a listed provider')
        : more
          ? `${named.join(', ')} or another listed provider`
          : `${named.slice(0, -1).join(', ')} or ${named[named.length - 1]}`;
    return `Paste a key from ${list}; Biorouter detects which.`;
  },
  detect: 'Detect provider',
  detecting: 'Detecting…',
  detected: (provider: string) => `Detected ${provider}`,
  detectedDetail: (model: string, count: number) => `${model} · ${count} models available`,
  errors: {
    timeout: {
      title: 'Detection timed out',
      detail: 'The provider took too long to respond. Check your connection and try again.',
    },
    network: {
      title: 'Could not reach the provider',
      detail: 'A network error occurred while validating the key. Check your connection.',
    },
    configuration: {
      title: 'Could not save provider',
      detail: 'The key was valid, but Biorouter could not save the provider configuration.',
    },
    invalidKey: {
      title: 'Key was rejected',
      detail:
        "The key matched a provider but was rejected. Check that it's complete, active and has credit.",
    },
    noMatch: (supported: string) => ({
      title: 'Could not detect provider',
      detail: `This key didn't match a supported provider (${supported}). Pick yours from the list below.`,
    }),
  },
} as const;

/**
 * Ollama's panel (`OllamaInlineCard`). Sentence case, and no exclamation marks in a toast.
 *
 * The models are the daemon's recommendations (`GET /ollama/recommended`), never a tag written
 * here. Every status names a host the user chose, so "not detected" says where it looked.
 */
export const ollamaCopy = {
  checking: 'Checking for Ollama…',
  checkingModels: 'Checking for models…',
  /** "Ollama not detected", or "Ollama not detected at http://127.0.0.1:11500" for a chosen host. */
  notDetected: (host?: string) => `Ollama not detected${host ? ` at ${host}` : ''}`,
  /**
   * "Ollama running · qwen3.8:27b ready". The model part waits until Ollama has said what it has
   * pulled; before that the status says only that the server answers.
   */
  running: (host?: string, model?: { tag: string; installed: boolean }) =>
    `Ollama running${host ? ` at ${host}` : ''}${
      model ? ` · ${model.tag} ${model.installed ? 'ready' : 'not installed'}` : ''
    }`,
  /** A server this screen cannot ask (the CSP admits only 127.0.0.1): this computer's, at ::1. */
  atHost: (host: string) => `Ollama at ${host}`,
  /** The same, on another computer. */
  onAnotherComputer: (host: string) => `Ollama on another computer · ${host}`,
  remoteNote: (host: string) =>
    `Biorouter is set to use the Ollama server at ${host}. This screen can only check an Ollama at 127.0.0.1, so it cannot see which models that server has. Pull the model there first, then connect.`,
  pullCommand: (tag: string) => `ollama pull ${tag}`,
  /** The daemon answered with nothing usable; a refusal is shown in its own words instead. */
  noRecommendations: 'Biorouter could not read its recommended Ollama models.',
  tryAgain: 'Try again',
  model: 'Model',
  /**
   * An option of the Model picker: "Qwen3.8 27B (recommended) · ~18 GB · Needs download", the
   * Llama Server picker's words. A server this screen cannot ask has no state word: it is not
   * guessed.
   */
  recommended: (name: string) => `${name} (recommended)`,
  approxSize: (gb: number) => `~${gb} GB`,
  installed: 'Installed',
  needsDownload: 'Needs download',
  install: 'Install Ollama',
  waiting: 'Waiting for Ollama to start…',
  /** The model's name is in the picker above; the button says what it does and its size. */
  downloadModel: (gb: number) => `Download model (~${gb} GB)`,
  downloading: (tag: string) => `Downloading ${tag}…`,
  startingDownload: 'Starting download…',
  use: 'Use Ollama',
  connecting: 'Connecting…',
  detectedToast: 'Ollama is running',
  detectedToastDetail: 'You can connect to it now.',
  downloadedToast: 'Model downloaded',
  downloadedToastDetail: (name: string, tag: string) => `${name} (${tag}) is ready in Ollama.`,
  downloadFailed: 'Download failed',
  /** Ollama's own reason: a failed pull used to be reported as a download. */
  downloadFailedDetail: (tag: string, reason: string) =>
    `Ollama could not download ${tag}: ${reason}`,
  connectedToast: 'Connected to Ollama',
  connectedToastDetail: (model: string) => `Chats start on ${model}.`,
  connectFailed: 'Connection failed',
  connectFailedDetail: (reason: string) => `Could not connect to Ollama: ${reason}`,
} as const;
