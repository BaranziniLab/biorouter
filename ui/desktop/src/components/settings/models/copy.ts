/**
 * The words of Settings > Models and its dialogs (codex simplicity redesign, spec §3.13).
 *
 * Tests import these instead of retyping them. A control's accessible name is a deliberate
 * choice here, not a side effect: `changeName` and `manageName` keep the names the daemon's
 * docs, the browser-surface tests and the focus-return tests already use, so a visible
 * "Change…" can sit on a button that is still announced as "Switch models".
 *
 * Help strings are InfoTip text: full sentences, at most two, about 160 characters. Labels and
 * one-line descriptions take no trailing period.
 */

export const defaultModelCopy = {
  section: 'Default model',
  sectionHelp:
    'New chats start on this model. A chat can use another one from its composer without changing this.',
  model: 'Model',
  change: 'Change…',
  /** The button's accessible name. Pinned by hostManagedSettings and the focus-return tests. */
  changeName: 'Switch models',
  providers: 'Providers',
  manage: 'Manage…',
  /** The button's accessible name. Pinned by hostManagedSettings. */
  manageName: 'Configure providers',
} as const;

export const localModelsCopy = {
  section: 'Local models',
  sectionHelp:
    'Models that run on this computer, so chats with them never leave it. Each one downloads once.',
  /** "2 of 7 installed", the section's count. */
  installedCount: (installed: number, total: number) => `${installed} of ${total} installed`,
  refresh: 'Refresh local models',
  install: 'Install',
  /** The quiet status an installed row shows at rest; the ⋯ takes its place on hover. */
  installed: 'Installed',
  installing: 'Installing',
  more: (name: string) => `More actions for ${name}`,
  viewInfo: 'View info',
  warmUp: 'Warm up',
  delete: 'Delete',
  retry: 'Retry',
  /** The one warning word a row shows, and only when the model is bigger than the machine's memory. */
  largeForThisMac: 'Large for this Mac',
  largeForThisComputer: 'Large for this computer',
  /** Row facts joined with a middle dot: `4.2 GB · 128k context`. */
  contextWindow: (tokens: number | null | undefined) => {
    if (typeof tokens !== 'number' || !Number.isFinite(tokens) || tokens <= 0) return null;
    const thousands = Math.round(tokens / 1024);
    return `${thousands.toLocaleString()}k context`;
  },
  close: 'Close',
  warmUpModel: 'Warm up model',
  installModel: 'Install model',
  /** A model's state, in words, for the info dialog's Status row. */
  status: {
    downloadedInOllama: 'Downloaded in Ollama',
    downloaded: 'Downloaded',
    fallbackReady: 'Fallback ready',
    partial: 'Partial download',
    needsDownload: 'Needs download',
  },
  /** The info dialog's rows. The dialog is where the machine facts live, not the row. */
  detail: {
    status: 'Status',
    family: 'Family',
    download: 'Download',
    expectedSpeed: 'Expected speed',
    context: 'Context',
    minimumMemory: 'Minimum memory',
    recommendedMemory: 'Recommended memory',
    detectedMemory: 'Detected memory',
    recommendation: 'Recommendation',
    ollamaModel: 'Ollama model',
    fallbackGguf: 'Fallback GGUF',
    officialUrl: 'Official URL',
    modelStore: 'Model store',
    modelBlob: 'Model blob',
  },
  /** A model bigger than this computer's memory recommendation asks before it installs or loads. */
  confirmResources: {
    title: 'Continue anyway?',
    installAnyway: 'Install anyway',
    warmUpAnyway: 'Warm up anyway',
    fallbackConfirm: 'Continue',
    cancel: 'Cancel',
  },
  confirmDelete: {
    title: 'Delete this local model?',
    message: (name: string) =>
      `${name} will be removed from the local model inventory. You can download it again later.`,
    confirm: 'Delete',
    cancel: 'Cancel',
  },
} as const;

export const modelsDangerCopy = {
  section: 'Danger zone',
  row: 'Reset provider and model',
  rowHelp: 'Clears the provider and model new chats start on, so you can choose them again.',
  open: 'Reset…',
  confirmTitle: 'Reset provider and model?',
  confirmMessage:
    'New chats will have no provider or model until you choose them again. If no defaults are available, Biorouter opens the welcome screen.',
  confirm: 'Reset',
  cancel: 'Cancel',
} as const;

export const leadWorkerCopy = {
  /** The dialog's title and the switch's name: pinned by the tests and the composer chip. */
  title: 'Lead/worker mode',
  mode: 'Lead/worker mode',
  modeHelp:
    'A stronger lead model plans and recovers from failures, and a faster worker model does the routine steps.',
  leadModel: 'Lead model',
  leadModelHelp: 'The strong model for initial planning and for recovery after failures.',
  workerModel: 'Worker model',
  workerModelHelp: 'The fast model for routine execution steps.',
  leadPlaceholder: 'Select lead model…',
  workerPlaceholder: 'Select worker model…',
  customPlaceholder: 'Type model name here',
  backToList: 'Back to model list',
  leadTurns: 'Initial lead turns',
  leadTurnsHelp: 'How many turns the lead model takes at the start.',
  failureThreshold: 'Failure threshold',
  failureThresholdHelp: 'How many failures in a row hand the work back to the lead model.',
  fallbackTurns: 'Fallback turns',
  fallbackTurnsHelp: 'How many turns the lead model takes after a fallback.',
  loading: 'Loading…',
  cancel: 'Cancel',
  /** The button's name: pinned by hostManagedSettings and the dialog's tests. */
  save: 'Save settings',
} as const;

export const switchModelCopy = {
  /** The dialog's title: pinned by the Settings focus-return test and the browser-surface tests. */
  title: 'Switch models',
  provider: 'Provider',
  providerPlaceholder: 'Provider, type to search',
  model: 'Model',
  modelPlaceholder: 'Select a model, type to search',
  loadingModels: 'Loading models…',
  customModelName: 'Custom model name',
  customPlaceholder: 'Type model name here',
  backToList: 'Back to model list',
  recommended: 'Recommended',
  quickStart: 'Quick start guide',
  cancel: 'Cancel',
  /** The confirm's name: pinned by every SwitchModelModal test. */
  select: 'Select model',
  switching: 'Switching…',
} as const;
