import type { ChromeImportRequest } from '../types';
import type { Platform } from './profiles';

/**
 * The main process's own confirmation of a Chrome import (BB-21, S3 of the 2026-11-20 fix round).
 *
 * The renderer's dialog is where the person chooses, but a renderer is a web page: a compromised
 * or buggy one could call the import's IPC handler on its own. So before the import reads any
 * cookie, key, bookmark, history entry or preference, Electron main shows a native dialog it owns,
 * which no page can answer, naming the profile and what will be imported. Only **Import** there
 * starts the import.
 */

export type ImportConfirmationText = {
  message: string;
  detail: string;
  /** The button that starts the import; the other is Cancel. */
  confirmLabel: string;
};

/** Asks the person; resolves `true` only when they chose to import. */
export type ConfirmImport = (text: ImportConfirmationText) => Promise<boolean>;

/** What the native dialog says for `request`, in the import's own words. */
export function importConfirmation(
  request: ChromeImportRequest,
  profileName: string,
  platform: Platform
): ImportConfirmationText {
  const items: string[] = [];
  if (request.logins) {
    const sites = request.sites;
    items.push(
      sites === null
        ? 'Logins (cookies) for every site'
        : `Logins (cookies) for ${sites.length === 1 ? '1 site' : `${sites.length.toLocaleString('en-US')} sites`}`
    );
  }
  if (request.bookmarks) items.push('Bookmarks');
  if (request.history) items.push('History from the last 90 days');
  if (request.settings) items.push('Settings: search engine, languages and default zoom');
  const lines = [
    'Biorouter will copy into its own browser:',
    ...items.map((item) => `• ${item}`),
    '',
    'Saved passwords and payment data are never imported.',
  ];
  if (request.logins && platform === 'darwin') {
    lines.push('macOS will then ask to allow access to Chrome Safe Storage.');
  }
  return {
    message: `Import from Chrome (${profileName})?`,
    detail: lines.join('\n'),
    confirmLabel: 'Import',
  };
}
