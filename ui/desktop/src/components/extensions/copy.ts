/**
 * Every visible string the Extensions page and its failure notice render, in one place.
 *
 * Tests import these rather than restating them (Crew's `copy.ts` pattern; implementation spec
 * section 1, copy rules): sentence case, "Biorouter", "chat", the typographic ellipsis, no dashes
 * as punctuation. The row, the group labels and the configure dialogs have their own file in
 * `settings/extensions/copy.ts`.
 */
export const EXTENSIONS_COPY = {
  title: 'Extensions',
  /** The band's InfoTip: what the page is for. The shortcut lives in the filter's tooltip. */
  info: 'Extensions add tools, prompts and resources. Enabled ones apply to every new chat.',

  /** The band's one primary action and the three ways an extension arrives. */
  add: 'Add',
  browse: 'Browse marketplace',
  fromFile: 'Install from file (.brxt)',
  custom: 'Custom extension…',

  /** The dialog the custom item opens, and its submit button. */
  customTitle: 'Add custom extension',
  customSubmit: 'Add extension',
} as const;

/** The standing notice for an extension that failed to load. */
export const LOAD_FAILURE_COPY = {
  failedToLoad: (name: string) => `${name} failed to load`,
  askBiorouter: 'Ask Biorouter',
  copyError: 'Copy error',
  copied: 'Copied',
  dismiss: 'Dismiss',
} as const;
