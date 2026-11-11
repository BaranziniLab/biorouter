/**
 * Every sentence and label the Dictionary page shows, one source (DD-67, DD-68,
 * "The left sidebar Dictionary page" in `docs/dictation/dictionary.md`).
 *
 * The sentences are the message catalog in `docs/dictation/surfaces.md`
 * ("Message catalog"), word for word; `copy.test.ts` reads that table and fails
 * when a row here differs from it. A pull request that needs a new sentence adds
 * its row to the catalog first, then here. Placeholders keep the catalog's
 * spelling (`<term>`, `<n>`) and are filled with {@link dictionaryMessage}.
 *
 * Most refusals arrive from the daemon with their sentence already in `message`
 * (`routes/dictionary.rs`), so the page shows that text rather than a copy of it.
 * The validation sentences are here for the import plan, whose skipped lines
 * carry only a code.
 *
 * Labels (buttons, headings, menu items) are not sentences and are not in the
 * catalog.
 */
import { fillMessage } from '../dictation/copy';

export const DICTIONARY_MESSAGES = {
  'MSG-PUBLICIZE-WARN':
    'This term came from a private chat. Making it public lets it appear when you dictate into chats on public models.',
  'MSG-PUBLICIZE-WARN-NO-CHAT':
    'This term is private. Making it public lets it appear when you dictate into chats on public models.',
  'MSG-ROW-PRIVATE': 'Used only in private chats',
  'MSG-ROW-PRIVATE-INSTITUTION':
    'Used only in private chats on <institution> models and on local models',
  'MSG-ROW-PRIVATE-LOCAL': 'Used only in private chats on local models',
  'MSG-EXPORT-PRIVATE': 'This file leaves Biorouter. It will include your private terms.',
  'MSG-EMPTY':
    'No terms yet. Add the words dictation should spell your way, such as UCSF or EPAS1.',
  'MSG-TERM-TOO-LONG': 'Use a term of at most 64 characters.',
  'MSG-TERM-WORDS': 'Use a term of at most six words.',
  'MSG-TERM-PLAIN': 'Use one line of plain text.',
  'MSG-TERM-ADDRESS': 'A dictionary term cannot be a web address, an email address or a file path.',
  'MSG-TERM-IDENTIFIER': 'This looks like an identifier or a key, so it was not added.',
  'MSG-TERM-FORMS': 'Use at most eight spoken forms and sixteen misrecognitions for a term.',
  'MSG-DICTIONARY-CAP':
    'Your dictionary holds 5,000 active terms here, the most dictation can use. Delete terms you no longer dictate, then add this one.',
  'MSG-DICTIONARY-STALE': 'This term changed since you opened it. Reload it and try again.',
  'MSG-IMPORT-ROW-UNREADABLE': 'This row could not be read.',
  'MSG-DICTIONARY-HELP':
    'Words dictation should spell your way, such as names, gene symbols and acronyms. A private term is used only when you dictate into a private chat.',
  'MSG-DICTIONARY-NO-MATCH': 'No terms match this filter.',
  'MSG-DELETE-TERM': 'Dictation stops using <term>, and Biorouter does not suggest it again.',
  'MSG-DELETE-ALL':
    'This deletes the <n> terms listed here, and Biorouter does not suggest them again. It cannot be undone.',
  'MSG-DICTIONARY-TOO-LARGE':
    'That list is larger than 1 MB or 5,000 rows. Split it and add each part.',
  'MSG-ADD-TERMS-HELP':
    'Paste one term per line, such as UCSF or EPAS1 = epas one, or drop a .txt, .csv, .tsv or .json file. Terms you already have gain the new spoken forms; nothing is deleted.',
  'MSG-IMPORT-PLAN': '<new> new · <existing> already in your dictionary · <skipped> skipped',
  'MSG-IMPORT-SKIPPED': 'Line <line>: <sentence>',
  'MSG-SPOKEN-FORMS-HELP':
    'How you say the term, one way per line, at most eight. Leave it empty when dictation already hears the term.',
  'MSG-MATCH-HELP':
    'Also fix near misses corrects close misspellings, and Exact spelling only corrects only what you listed. Help recognition only never changes the text after transcription.',
  'MSG-REVIEW-HELP':
    'An accepted term starts shaping dictation. A dismissed one is not suggested again.',
  'MSG-LEARN-FROM-CHATS-HELP':
    'Biorouter looks for terms in your messages, on this computer, and suggests them for you to accept. It starts off because how well it picks terms is not checked yet.',
  'MSG-PRIVATIZE-CONFIRM':
    'Dictation then uses this term only when you dictate into private chats.',
  'MSG-EXPORT-SAVED': 'Saved <file> to <folder>.',
  'MSG-EXPORT-WRITE-FAILED': 'The file could not be written to <folder>. Nothing was saved.',
} as const;

export type DictionaryMessageId = keyof typeof DICTIONARY_MESSAGES;

/**
 * The two or more form of MSG-ROW-PRIVATE-INSTITUTION, which the catalog writes
 * in that row's note ("a row that names two or more institutions reads …").
 */
export const ROW_PRIVATE_INSTITUTIONS =
  'Tied to <institutions>; used only in private chats on local models';

/** A catalog sentence by id, filled. */
export function dictionaryMessage(
  id: DictionaryMessageId,
  values: Readonly<Record<string, string>> = {}
): string {
  return fillMessage(DICTIONARY_MESSAGES[id], values);
}

/**
 * The sentence for a validation code from the store (`store/validate.rs`), for
 * the import plan's skipped lines. `empty` reads as MSG-TERM-PLAIN, as the
 * store's own `ValidationCode::message` does.
 */
export function validationSentence(code: string): string | null {
  switch (code) {
    case 'too_long':
      return DICTIONARY_MESSAGES['MSG-TERM-TOO-LONG'];
    case 'too_many_words':
      return DICTIONARY_MESSAGES['MSG-TERM-WORDS'];
    case 'empty':
    case 'line_break':
    case 'control_character':
    case 'angle_bracket':
      return DICTIONARY_MESSAGES['MSG-TERM-PLAIN'];
    case 'url':
    case 'email':
    case 'path':
      return DICTIONARY_MESSAGES['MSG-TERM-ADDRESS'];
    case 'digits_only':
    case 'long_hex':
    case 'secret_shaped':
      return DICTIONARY_MESSAGES['MSG-TERM-IDENTIFIER'];
    case 'too_many_forms':
      return DICTIONARY_MESSAGES['MSG-TERM-FORMS'];
    case 'cap':
      return DICTIONARY_MESSAGES['MSG-DICTIONARY-CAP'];
    case 'unreadable':
      return DICTIONARY_MESSAGES['MSG-IMPORT-ROW-UNREADABLE'];
    default:
      return null;
  }
}

/** Institutions joined the catalog's way: "UCSF", "UCSF and Stanford", "A, B and C". */
export function joinInstitutions(names: readonly string[]): string {
  if (names.length <= 1) return names[0] ?? '';
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
}

export const DICTIONARY_LABELS = {
  title: 'Dictionary',
  filter: 'Filter terms',
  export: 'Export…',
  add: 'Add',
  addTerm: 'Add a term…',
  pasteList: 'Paste a list…',
  importFile: 'Import a file…',
  pageMenu: 'More dictionary actions',
  deleteAll: 'Delete all terms…',
  groups: {
    suggested: 'Suggested',
    agent: 'Added by the agent',
    user: 'Added by you',
    chats: 'From your chats',
    imported: 'Imported',
    matches: 'Matches',
  },
  // Under an empty **Added by you**, which shows whenever the dictionary holds any row: a
  // dictionary the agent started has rows that are not the person's yet.
  userGroupEmpty: 'Terms you add show here.',
  reviewAll: 'Review all…',
  agentSettings: {
    title: "Agent's terms",
    suggest: 'Wait for my review',
    onRequest: 'Add when I ask, suggest otherwise',
    add: 'Add without asking',
    /** `learn_from_chats`, the chat extractor's switch (PR-24, DD-57). */
    learnFromChats: 'Also find terms in my messages automatically',
  },
  sources: {
    user: 'Added by you',
    import: 'Imported',
    agent: 'Added by the agent',
    auto: 'Found in your chats',
    correction: 'Learned from your corrections',
  },
  saidAs: (forms: readonly string[]) => `Said as ${forms.join(', ')}`,
  uses: (n: number) => (n === 1 ? '1 use' : `${n.toLocaleString('en-US')} uses`),
  notUsed: 'Not used yet',
  privateTerm: 'Private term',
  edit: 'Edit',
  editMenu: 'Edit…',
  copyTerm: 'Copy term',
  copied: 'Term copied',
  openChat: 'Open the chat it came from',
  makePublic: 'Make public…',
  makePrivate: 'Make private…',
  delete: 'Delete…',
  accept: 'Accept',
  dismiss: 'Dismiss',
  keep: 'Keep',
  remove: 'Remove',
  moreActions: (term: string) => `More actions for ${term}`,
  rowAction: (action: string, term: string) => `${action} ${term}`,
  settled: {
    accepted: 'Accepted',
    dismissed: 'Dismissed',
    deleted: 'Deleted',
    kept: 'Kept',
    removed: 'Removed',
  },
  cancel: 'Cancel',
  save: 'Save',
  done: 'Done',
  term: {
    addTitle: 'Add a term',
    editTitle: 'Edit term',
    term: 'Term',
    spokenAs: 'Spoken as',
    note: 'Note',
    onlyPrivate: 'Only in private chats',
    moreOptions: 'More options',
    match: 'Match',
    matchOptions: {
      exact: 'Exact spelling only',
      fuzzy: 'Also fix near misses',
      prompt_only: 'Help recognition only',
    },
    always: 'Always use in dictation',
    add: 'Add term',
    adding: 'Adding…',
    saving: 'Saving…',
  },
  addTerms: {
    title: 'Add terms',
    field: 'Terms',
    chooseFile: 'Choose a file…',
    checking: 'Checking…',
    apply: 'Add terms',
    applying: 'Adding…',
  },
  review: {
    title: 'Review suggestions',
    selectAll: 'Select all',
    acceptSelected: 'Accept selected',
    dismissSelected: 'Dismiss selected',
  },
  exportDialog: {
    title: 'Export your dictionary',
    withPrivate: 'Export with private terms',
    publicOnly: 'Export public terms only',
    saveTitle: 'Export dictionary',
    fileType: 'JSON',
    saved: 'Dictionary exported',
    failed: "Couldn't export the dictionary",
  },
  publicize: {
    title: 'Make this term public?',
    confirm: 'Make public',
  },
  privatize: {
    title: 'Make this term private?',
    confirm: 'Make private',
  },
  deleteTerm: {
    title: (term: string) => `Delete ${term}?`,
    confirm: 'Delete',
  },
  deleteAllDialog: {
    title: 'Delete all terms?',
    field: 'Type delete to confirm',
    confirm: 'Delete all terms',
  },
  /** The link from the setup popover and Settings to this page. */
  editYourDictionary: 'Edit your dictionary',
  waiting: (n: number) => (n === 1 ? '1 suggestion waiting' : `${n} suggestions waiting`),
} as const;

/** The file the export writes, as `GET /dictionary/export` names it. */
export const DICTIONARY_EXPORT_FILE = 'biorouter-dictionary.json';
