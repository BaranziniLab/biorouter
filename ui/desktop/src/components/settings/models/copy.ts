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
  checking: 'Checking local models…',
  /** "2 of 7 installed", the section's count. */
  installedCount: (installed: number, total: number) => `${installed} of ${total} installed`,
  refresh: 'Refresh local models',
  install: 'Install',
  installing: 'Installing',
  more: (name: string) => `More actions for ${name}`,
  viewInfo: 'View info',
  warmUp: 'Warm up',
  warmingUp: 'Warming up',
  delete: 'Delete',
  deleting: 'Deleting',
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
