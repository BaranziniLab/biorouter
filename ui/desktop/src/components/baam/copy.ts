/**
 * Every visible string the two BAAM browse dialogs render, in one place.
 *
 * Tests import these rather than restating them, so a wording change is one
 * edit here and the assertions follow (Crew's `copy.ts` pattern; implementation
 * spec §1, copy rules). Sentence case, "Biorouter", the typographic ellipsis.
 *
 * Strings the registry or the installer own (`registry.ts`, `installCopy.ts`)
 * stay where they are: they are shared with other surfaces.
 */
export const MARKETPLACE_COPY = {
  /** The one visible line under a browse dialog's title. */
  subtitle: 'From the Biorouter marketplace',
  /** Placeholder of the search field. Its accessible name is per dialog. */
  searchPlaceholder: 'Search',
  loading: 'Loading catalog…',
  loadError: 'Could not load the marketplace catalog.',
  installed: 'Installed',
} as const;

export const BROWSE_EXTENSIONS_COPY = {
  title: 'Browse extensions',
  help: 'Add one extension at a time. Most ask for credentials during install.',
  searchLabel: 'Search extensions',
  empty: 'No extensions match your search.',
  add: 'Add',
  configure: 'Configure',
} as const;

export const BROWSE_SKILLS_COPY = {
  title: 'Browse skills',
  help: 'Select as many skills as you like. Skills need no setup.',
  searchLabel: 'Search skills',
  empty: 'No skills match your search.',
  filterLabel: 'Category',
  all: 'All',
  /** Counts ROWS, on purpose: a package is one box. */
  selectAll: (n: number) => `Select all (${n})`,
  clearSelection: 'Clear selection',
  selected: (n: number) => `${n} selected`,
  matches: 'Matches',
  cancel: 'Cancel',
  installing: 'Installing…',
} as const;
