import type { ChromeImportLogins, ChromeImportSummary } from '../../../browser/types';

/** The words of the Chrome import (BB-21 to BB-25). */
export const CHROME_IMPORT_COPY = {
  title: 'Import from Chrome',
  open: 'Import from Chrome…',
  subtitle: 'Bring your logins, bookmarks and history into the Biorouter Browser.',
  offerTitle: 'Use your Chrome logins and bookmarks here?',
  notNow: 'Not now',
  profile: 'Chrome profile',
  logins: 'Logins',
  loginsHelp: 'The cookies that keep you signed in to sites. Passwords are never imported.',
  bookmarks: 'Bookmarks',
  history: 'History',
  historyHelp: 'Pages from the last 90 days, for suggestions in the address field.',
  settings: 'Settings',
  settingsHelp: 'Search engine, languages and default zoom.',
  notImported: 'Saved passwords and payment data stay in Chrome.',
  notImportedLabel: 'What is not imported',
  notImportedHelp:
    'The import brings sessions, not credentials. Passwords and cards stay with Chrome and your password manager, and the cache refills as you browse.',
  windowsLogins:
    'Chrome on Windows locks its logins to Chrome, so other apps cannot import them. Log in on each site in the Biorouter Browser instead.',
  macPrompt: 'macOS will ask to allow access to Chrome Safe Storage. Choose Allow.',
  sites: 'Choose sites',
  sitesSummary: (chosen: number, total: number) =>
    chosen === total ? `All ${total} sites` : `${chosen} of ${total} sites`,
  sitesFilter: 'Filter sites',
  sitesUnreadable: 'Chrome’s cookie file could not be read.',
  selectAll: 'Select all',
  selectNone: 'Select none',
  cookieCount: (count: number) => (count === 1 ? '1 cookie' : `${count} cookies`),
  cancel: 'Cancel',
  import: 'Import',
  importing: 'Importing…',
  done: 'Done',
  summaryTitle: 'Imported from Chrome',
  remove: 'Remove imported data',
  removeNamed: (profile: string) => `Remove data imported from Chrome (${profile})`,
  removing: 'Removing…',
  removed: 'Imported data removed.',
  section: 'Browser',
  sectionHelp: 'The Biorouter Browser in the side panel.',
  importRow: 'Import from Chrome',
  importRowHelp: 'Logins, bookmarks, history and a few settings, only when you ask.',
  notFound: 'Chrome was not found on this computer.',
  nothingChosen: 'Choose something to import.',
  noneSelected: 'Choose at least one site, or turn off Logins.',
} as const;

const LOGINS_SENTENCE: Record<Exclude<ChromeImportLogins, 'imported' | 'not-chosen'>, string> = {
  'none-found': 'There were no logins to import.',
  unreadable: 'Logins were not imported because Chrome’s cookie file could not be read.',
  unsupported: CHROME_IMPORT_COPY.windowsLogins,
  'not-allowed': 'Logins were not imported because access was not allowed.',
  'not-found': 'Logins were not imported because Chrome’s key was not found.',
  'key-store-off': 'Logins were not imported because the system key store is turned off.',
  'not-from-app':
    'Logins were not imported because only an installed Biorouter app can read Chrome’s key.',
  failed: 'Logins were not imported because Chrome’s key could not be read.',
};

const number = (value: number) => value.toLocaleString('en-US');
const plural = (value: number, one: string, many: string) =>
  `${number(value)} ${value === 1 ? one : many}`;

/** One line per thing imported, for the summary and Settings. */
export function summaryParts(summary: ChromeImportSummary): string[] {
  const parts: string[] = [];
  if (summary.logins === 'imported' && summary.cookies > 0) {
    parts.push(`logins for ${plural(summary.cookieSites, 'site', 'sites')}`);
  }
  if (summary.bookmarks !== null && summary.bookmarks >= 0) {
    parts.push(plural(summary.bookmarks, 'bookmark', 'bookmarks'));
  }
  if (summary.history !== null && summary.history >= 0) {
    parts.push(plural(summary.history, 'history entry', 'history entries'));
  }
  if (summary.settings && summary.settings.length > 0) parts.push(summary.settings.join(', '));
  return parts;
}

/** Why logins are missing from a summary, or `null` when nothing needs saying. */
export function loginsNote(summary: ChromeImportSummary): string | null {
  if (summary.logins === 'imported' || summary.logins === 'not-chosen') return null;
  return LOGINS_SENTENCE[summary.logins];
}

/** "Imported from Chrome (Work) on Nov 19": the one line Settings shows for an import. */
export function summaryLine(summary: ChromeImportSummary): string {
  const date = new Date(summary.importedAt).toLocaleDateString('en-US', {
    month: 'short',
    day: 'numeric',
  });
  return `Imported from Chrome (${summary.profileName}) on ${date}`;
}

/**
 * "Logins for 96 sites, 412 bookmarks, 8,900 history entries, search engine.": what the import
 * brought, and why logins are missing when they are, for the line's InfoTip.
 */
export function summaryDetails(summary: ChromeImportSummary): string {
  const parts = summaryParts(summary);
  const brought = parts.length > 0 ? parts.join(', ') : 'nothing';
  const sentence = `${brought[0].toUpperCase()}${brought.slice(1)}.`;
  const note = loginsNote(summary);
  return note ? `${sentence} ${note}` : sentence;
}
