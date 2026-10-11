/**
 * Every string Built apps shows, in one place (Crew's `copy.ts` pattern). Tests import these
 * rather than restating them.
 *
 * Copy rules (spec section 1): sentence case, "chat" not "conversation", "Biorouter", the
 * typographic ellipsis, no dashes as punctuation. "Agent Drafter" keeps its capitals.
 */

const plural = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`;

export const appsCopy = {
  title: 'Built apps',
  info: 'Apps made with Agent Drafter. Each runs its own agent and opens in your browser.',
  refresh: 'Refresh',
  filterLabel: 'Filter built apps',

  emptyTitle: 'No apps built yet',
  emptyDescription: 'Ask Biorouter to build one with Agent Drafter.',
  noMatchTitle: 'No matching apps',
  noMatchDescription: 'Try another search.',
  loadFailed: (reason: string) => `Couldn’t load apps: ${reason}`,
  retry: 'Retry',

  // Row actions. Names carry the app, so ten rows' buttons can be told apart.
  launch: 'Launch',
  launchNamed: (title: string) => `Launch ${title} in browser`,
  openChat: 'Open the chat it was built in',
  exportItem: 'Export…',
  // A menu item that opens a confirmation takes the ellipsis, as Skills' Delete… does (V3-21).
  delete: 'Delete…',
  moreActionsNamed: (title: string) => `More actions for ${title}`,

  updated: (when: string) => `Updated ${when}`,
  created: (when: string) => `Created ${when}`,
  /** The surface a v2 app declares, for its kind badge's help. */
  surface: (actions: number, signals: number) =>
    [
      ...(actions > 0 ? [plural(actions, 'action')] : []),
      ...(signals > 0 ? [plural(signals, 'signal')] : []),
    ].join(' · '),

  deleteTitle: (title: string) => `Delete ${title}?`,
  // A destructive consequence: stays visible at the moment of decision (principle 2).
  deleteMessage:
    'This permanently removes the app and its files from disk. This action cannot be undone.',
  deleteConfirm: 'Delete',
  cancel: 'Cancel',

  // Toasts: one line. The title is the app; the reason rides the message.
  savedTo: (dir: string) => `Saved to ${dir}`,
  deleted: (title: string) => `Deleted ${title}`,
  couldNotOpen: 'Couldn’t open the app in your browser.',
  nothingExported: 'Nothing was exported.',
  couldNotExport: 'Couldn’t export the app.',
  couldNotDelete: (reason?: string) =>
    reason ? `Couldn’t delete the app: ${reason}` : 'Couldn’t delete the app.',

  exportDialog: {
    title: (title: string) => `Export ${title}`,
    // The safety fact stays visible (Crew: never hide security state).
    credentials: 'Credentials are never included.',
    mode: 'Export mode',
    launcher: 'Launcher',
    launcherHint: 'Smallest. Uses what the other machine has.',
    full: 'Full',
    fullHint: 'Includes the items below. Can be large.',
    knowledgeBase: 'Knowledge base',
    knowledgeBaseInfo:
      'Bundled with its sources as a .brkb file and imported on the other machine on first run.',
    skills: 'Skills',
    skillsInfo: 'Staged in the export and installed into the other machine’s skills on first run.',
    extensions: 'Extensions',
    extensionsInfo:
      'Bundled or pinned for install on first run. Built-in extensions already travel with Biorouter.',
    includeBiorouter: 'Include Biorouter (~110 MB)',
    includeBiorouterInfo: 'Skips the Biorouter download on the other machine.',
    cancel: 'Cancel',
    export: 'Export',
  },
} as const;
