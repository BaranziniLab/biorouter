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
  openSubagentRun: 'Open the subagent run it was built in',
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
  deleted: (title: string) => `Deleted ${title}`,
  couldNotOpen: 'Couldn’t open the app in your browser.',
  couldNotExport: 'Couldn’t export the app.',
  couldNotDelete: (reason?: string) =>
    reason ? `Couldn’t delete the app: ${reason}` : 'Couldn’t delete the app.',

  /**
   * Where the chat an app was built in leads (`buildChat.ts`, which decides the state). The
   * labels name the app so ten rows' controls can be told apart; the notes are the short form
   * the row's meta line and the unavailable menu item show; the refusals are a click's toast.
   */
  buildChat: {
    openLabel: (title: string) => `Open the chat where ${title} was built`,
    openSubagentLabel: (title: string) => `Open the subagent run that built ${title}`,
    deletedLabel: (title: string) => `The chat where ${title} was built was deleted`,
    replacedLabel: (title: string) => `The chat where ${title} was built no longer exists`,
    unreachableLabel: (title: string) => `The chat where ${title} was built can't be opened here`,
    noneLabel: (title: string) => `No chat was recorded for ${title}`,
    deletedNote: 'Build chat deleted',
    replacedNote: 'Build chat no longer exists',
    unreachableNote: "Build chat can't be opened here",
    noneNote: 'No build chat recorded',
    deletedRefusal: 'The chat where this app was built has been deleted.',
    replacedRefusal: 'The chat where this app was built no longer exists.',
    unreachableRefusal: "The chat where this app was built can't be opened here.",
    noneRefusal: 'No chat was recorded for this app.',
    unansweredRefusal: "Couldn't reach Biorouter to check that chat. Try again.",
    stillChecking: 'Biorouter was still checking that chat. Try again.',
    couldNotOpen: "Couldn't open the chat where this app was built.",
  },

  /**
   * An app whose manifest the background service could not read: it is listed so it can be
   * seen and deleted, and Delete is its only action.
   */
  broken: {
    badge: "Can't be read",
    message: "Biorouter can't read this app's manifest, so it can't be launched or exported.",
    deleteNamed: (title: string) => `Delete ${title}`,
  },

  exportDialog: {
    title: (title: string) => `Export ${title}`,
    // The safety fact stays visible (Crew: never hide security state).
    credentials: 'Credentials are never included.',
    // A `biorouter serve` browser downloads the export instead of writing a folder.
    browserSubtitle: 'Downloads the app as a .zip. Credentials are never included.',
    mode: 'Export mode',
    launcher: 'Launcher',
    launcherHint: 'Smallest. Uses what the other machine has.',
    full: 'Full',
    fullHint: 'Includes the items below. Can be large.',
    knowledgeBase: 'Knowledge base',
    knowledgeBases: 'Knowledge bases',
    knowledgeBaseInfo:
      'Bundled with its sources as a .brkb file. A private base is included only when you can open it.',
    skills: 'Skills',
    skillsInfo: 'Staged in the export and installed into the other machine’s skills on first run.',
    extensions: 'Extensions',
    extensionsInfo:
      'Listed in export.json for the other machine to install from the marketplace. Built-in extensions already come with Biorouter.',
    nothingToInclude: 'This app uses no knowledge bases, skills or extensions.',
    includeBiorouter: 'Include Biorouter (~110 MB)',
    includeBiorouterInfo: 'Runs on a computer like this one that has no Biorouter installed.',
    cancel: 'Cancel',
    export: 'Export',
    exporting: 'Exporting…',

    // The app check (harness lint) found errors: say so and offer to export anyway.
    lintTitle: (title: string) => `Export ${title} anyway?`,
    lintSubtitle: 'The app check found problems that can keep the exported app from working.',
    lintHelp: 'Fix them in the chat that built the app, or export it as it is.',
    problemsFound: 'Problems found',
    moreProblems: (count: number) => `${count} more`,
    exportAnyway: 'Export anyway',
  },
} as const;
