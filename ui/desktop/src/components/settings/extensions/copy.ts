/**
 * Every visible string the extension rows, the configure dialog and the saved-credentials dialog
 * render, in one place.
 *
 * Tests import these rather than restating them (Crew's `copy.ts` pattern; implementation spec
 * section 1, copy rules): sentence case, "Biorouter", "chat", the typographic ellipsis, no dashes
 * as punctuation. The page itself (`extensions/copy.ts`) keeps its own.
 *
 * ⚠ **Three strings are privacy disclosures (issue #56 section 13.5) and are quoted by the
 * design document, so they read exactly as written:** the three provenance sentences in
 * `baam/registry.ts`, and the two tier lines on the Add form
 * (`EXTENSION_MODAL_COPY.tierPrivate` and `.tierPublic`). They stay visible; only the built-in
 * sentence, which is not one of the three, lives in an InfoTip.
 */
export const EXTENSION_ROW_COPY = {
  /** The group labels. The caps style uppercases them; the count follows. */
  groups: {
    enabled: 'Enabled',
    available: 'Available',
  },

  /** The ⋯ menu behind a row (and the same items on right-click). */
  configure: 'Configure…',
  remove: 'Remove…',

  /** The Privacy badge's InfoTip name. */
  privacy: 'Privacy',

  /**
   * Not one of section 13.5's three strings: a built-in is neither published on the marketplace
   * nor installed from a file, so neither of those would be true of it. It sits in the privacy
   * badge's InfoTip (and, through the InfoTip, in a visually hidden node every row keeps).
   */
  builtInProvenance: 'Public: built into Biorouter, not on the marketplace. Any model can call it.',

  /**
   * Section 14.5's third state. The badge is the visible short form; the sentence (which names
   * the provider the card judged against) is the badge's help and its hidden text.
   */
  unavailable: 'Unavailable in new chats',
  unavailableHelp: (enabled: boolean, provider: string) =>
    `${enabled ? 'Enabled, but unavailable' : 'Unavailable'} in new chats (default model is public). Judged against your default provider, ${provider}.`,

  /** The marketplace catalogue's age, in the help of the first group's heading. */
  catalogName: 'Marketplace catalog',
  catalogNote: (freshness: string) => `Marketplace catalog · ${freshness}`,

  emptyTitle: 'No extensions yet',
  emptyDescription: 'Browse the marketplace or add one you already have.',
  noMatchTitle: 'No matching extensions',
  noMatchDescription: 'Try a different name or description.',

  /** The one-shot suggestion after Workspace Control is turned on. */
  chatrecallToastTitle: 'Workspace Control enabled',
  chatrecallToastMessage:
    'Turn on Chat Recall in Settings, Chat, Capabilities to search past chats too.',
} as const;

/** The configure and add dialog. */
export const EXTENSION_MODAL_COPY = {
  nameLabel: 'Name',
  descriptionLabel: 'Description',

  typeLabel: 'Type',
  typeHelp:
    'Command runs a program on this computer (stdio). HTTP connects to a server by its URL (streamable HTTP).',
  typeCommand: 'Command',
  typeHttp: 'HTTP',
  typeSse: 'SSE (unsupported)',

  commandLabel: 'Command',
  commandPlaceholder: 'npx -y @scope/server',
  endpointLabel: 'Endpoint',
  endpointPlaceholder: 'https://example.com/mcp',

  timeoutLabel: 'Timeout (seconds)',
  timeoutRequired: 'Enter a timeout in seconds.',

  nameRequired: 'Name is required',
  commandRequired: 'Command is required',
  endpointRequired: 'Endpoint URL is required',

  installationNotes: 'Installation notes',

  /**
   * Section 13.5: the Add form carries the same line the .brxt install modal does. It follows
   * the name as it is typed, because on this form the name is the whole input to the tier.
   */
  tierPrivate:
    'The Biorouter marketplace publishes this name as private, so this extension will be Private: only private models will be able to call it.',
  tierPublic:
    'Extensions you add by hand are always Public. Any model, including commercial models hosted outside your institution, will be able to call this extension.',

  envTitle: 'Environment variables',
  envHelp: 'Fill both fields, then Add. Saved secrets show as dots, and Edit replaces one.',
  headersTitle: 'Request headers',
  headersHelp: 'Sent with every request to the server. Fill both fields, then Add.',
  variableName: 'Variable name',
  headerName: 'Header name',
  value: 'Value',
  add: 'Add',
  edit: (key: string) => `Edit ${key}`,
  removeEntry: (key: string) => `Remove ${key}`,
  variableNeedsBoth: 'Both variable name and value must be entered',
  variableNoSpaces: 'Variable name cannot contain spaces',
  headerNeedsBoth: 'Both header name and value must be entered',
  headerNoSpaces: 'Header name cannot contain spaces',

  /** Removal keeps credentials; reviewing them is a separate step. */
  reviewCredentials: 'Review saved credentials',
  credentialsHelp:
    'Removing an extension keeps its saved credentials for reuse. You can review and delete unshared ones separately.',
  sharedCredentialsHelp: 'Credentials shared with other extensions or providers are kept.',

  removeTitle: (name: string) => `Remove "${name}"?`,
  removeDescription:
    'This will remove the extension configuration. Saved credentials are retained and may be reused automatically if you reinstall.',
  removeAction: 'Remove extension',
  confirmRemoval: 'Confirm removal',
  cancel: 'Cancel',

  unsavedTitle: 'Unsaved changes',
  unsavedMessage:
    'You have unsaved changes to the extension configuration. Are you sure you want to close without saving?',
  discard: 'Discard changes',
} as const;

/** The saved-credentials dialog behind "Review saved credentials". */
export const EXTENSION_CREDENTIALS_COPY = {
  title: (name: string) => `Saved credentials for ${name}`,
  subtitle: (name: string) => `Delete saved values for ${name}. Shared ones are kept.`,
  help: 'Only saved values listed here can be deleted. Environment variables, values in extension settings and credentials already loaded in running chats stay unchanged.',
  checking: 'Checking saved credentials…',
  none: 'No saved credential references for this extension.',
  noValue: 'No saved value',
  kept: (usedBy: string) => `Kept: used by ${usedBy}`,
  willDelete: 'Will be deleted from saved credentials',
  restart: 'Restart Biorouter after deleting credentials.',
  deleted: 'Saved credentials deleted. Restart Biorouter to apply.',
  deletedHelp:
    'You will need to enter them again when required, unless your environment or extension settings supply them.',
  loadFailed: 'Could not verify saved credentials. Close this dialog and try again.',
  deleteFailed:
    'Credentials could not be deleted. References may have changed or storage is unavailable. Close this dialog and review again.',
  cancel: 'Cancel',
  done: 'Done',
  deleting: 'Deleting…',
  deleteCount: (count: number) => `Delete ${count} saved credential${count === 1 ? '' : 's'}`,
} as const;
