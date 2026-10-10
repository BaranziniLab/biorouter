/**
 * The Knowledge view's own words (spec 3.12, copy rules in section 1). Tests import these rather
 * than retyping them, so a copy change is one edit.
 *
 * Sentence case, "chat" not "conversation", no dashes as punctuation. InfoTip text is at most two
 * sentences. The privacy disclosures and every error line stay visible wherever they are drawn
 * (principle 2); only explanations move into InfoTips.
 */
export const knowledgeCopy = {
  /** The band's `<h1>`; e2e `knowledge-ingest.spec.ts` finds the heading by this name. */
  title: 'Knowledge',
  /** The band's InfoTip, beside the title. */
  viewInfo: 'Personal knowledge bases Biorouter builds and maintains for you.',
  /** The band's subject when the chat has no primary base. */
  noPrimary: 'No primary knowledge base',
  /** The band's counts: "43 pages · 38 links". */
  summary: (pages: number, links: number) =>
    `${pages} ${pages === 1 ? 'page' : 'pages'} · ${links} ${links === 1 ? 'link' : 'links'}`,

  /** The narrow step's tab pair. */
  tabs: { sources: 'Sources', graph: 'Graph' },

  /** The band's actions. */
  actions: {
    check: 'Check for problems',
    refresh: 'Refresh graph',
    more: 'More knowledge base actions',
    export: 'Export as .brkb',
    changeLog: 'Change log',
    openFolder: 'Open folder',
    delete: 'Delete knowledge base',
  },

  /** The tier badge-button (Crew's ClassificationBadge) and its menu. */
  tier: {
    privateHint: 'Only private models can read or write this base',
    publicHint: 'Any model can read this base',
    makePublic: 'Make public…',
    makePrivate: 'Make private',
    /** The badge-button's whole accessible name, as one hidden run (Crew UXN-15). */
    name: (label: string, hint: string, enforcementOff: boolean) =>
      `${label}${enforcementOff ? ' (enforcement off)' : ''}: ${hint}`,
  },

  /** The format badge in the band. Legacy explains itself in an InfoTip. */
  legacyFormatName: 'Legacy format',

  /** The Sources rail's staging affordances. */
  ingest: {
    dropLabel: 'Drag and drop to stage',
    dropHint: 'PDF, Office, web pages, folders',
    dropInfoLabel: 'Which files can be staged',
    dropInfo:
      'Readable files (.pdf, .pptx, .xlsx, .docx, .csv, .md, .html, .txt) are staged one by one, and folders and archives are unpacked; binaries are skipped. Import a .brkb archive from Manage bases.',
    pasteText: 'Paste text',
    chooserTitle: 'Stage knowledge sources',
    chooserSubtitle: 'Add files, or a folder or archive.',
    chooseFiles: 'Choose files',
    chooseFolder: 'Choose folder or archive',
    pastePlaceholder: 'Paste text or links',
    pasteTitlePlaceholder: 'Title (optional)',
    stagedLabel: 'Staged',
    warnings: (n: number) => `${n} ${n === 1 ? 'warning' : 'warnings'}`,
    warningsInfoLabel: 'Curation warnings',
    warningsInfo:
      'Keep readable, high-signal sources. Leave out installers, giant raw exports, and files that are mostly IDs or machine noise.',
    /** The digest model picker's footer line, and its InfoTip. */
    modelUse: 'Used for digests, not chat',
    modelUseInfo:
      'This model digests staged sources and scheduled knowledge curation. Chat replies use the model chosen in the composer.',
    /** Why Digest does nothing yet, when there is nothing to fix: the button's Tooltip. */
    stageAFile: 'Stage a file to digest.',
    checkingModel: 'Checking which model this knowledge base digests with…',
    errorDetails: 'Error details',
  },

  /** The base switcher in the band, and its picker. */
  selector: {
    /** The trigger's accessible name: the base, and how many more this chat uses. */
    name: (base: string, others: number) =>
      `Knowledge base: ${base}${others > 0 ? `, and ${others} more in this chat` : ''}`,
    /** The "+n" count's Tooltip. */
    others: (n: number) => `${n} more knowledge ${n === 1 ? 'base' : 'bases'} in this chat`,
    notFollowing: 'Not following your default',
    useDefault: (name: string) => `Use ${name}`,
    useDefaultHint: (name: string) =>
      `This chat will use ${name}, and will keep following your default if it changes.`,
  },

  /** The base manager dialog. */
  manager: {
    title: 'Knowledge bases',
    /** One sentence, because it is the dialog's `aria-describedby` (ModalShell). */
    subtitle: 'Choose the bases this chat uses.',
    infoLabel: 'Knowledge bases in this chat',
    info: 'The primary is where knowledge is written when no base is named. Bases left out can still be named.',
    primary: 'Primary',
    copyId: 'Copy ID',
    copied: 'Copied the ID',
    copyFailed: 'Could not copy the ID',
  },

  /** The create dialog (format chooser). */
  format: {
    title: 'Create knowledge base',
    folder: (id: string) => `knowledge/${id}/`,
    folderLabel: 'Folder:',
    folderInfoLabel: 'Folder name',
    folderInfo: 'The id may change if it is taken.',
    permanent: 'You can’t change the format later.',
    okf: {
      name: 'General knowledge',
      line: 'General notes and project context',
      info: 'Any page type and any link name, and nothing is rejected. Validation reports broken links only.',
    },
    biookf: {
      name: 'Curated biomedical',
      line: 'Curated biomedical literature',
      info: '28 page types and 35 link predicates, checked; every link names its evidence and a primary source. Validation names the closest legal value.',
    },
  },
} as const;
