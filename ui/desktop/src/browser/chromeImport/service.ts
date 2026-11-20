import crypto from 'node:crypto';
import os from 'node:os';
import type { BrowserPreferences, BrowserPreferencesStore } from '../preferences';
import type { BrowserStore } from '../store';
import type {
  ChromeImportProfile,
  ChromeImportRequest,
  ChromeImportResult,
  ChromeImportSites,
  ChromeImportStatus,
} from '../types';
import { importConfirmation, type ConfirmImport } from './confirm';
import { cookieSite, type ElectronCookieDetails, type ImportedCookieKey } from './cookies';
import { planCookieSites, runChromeImport, type AppliedSettings, type ImportLog } from './importer';
import type { KeyProvider } from './keyProvider';
import { readManifest, summarize, writeManifest, type ImportRecord } from './manifest';
import type { ChromePreferences } from './profileData';
import { chromeProfileDir, readChromeProfiles, type Platform } from './profiles';

/**
 * The Chrome import as the main process runs it (BB-21 to BB-25): profiles, the per-site plan,
 * the import into the Biorouter Browser's own profile, the summary and removal.
 *
 * Electron is reached only through `deps` (the partition's cookie jar, the store, the settings),
 * so the Electron spec and the unit tests run this exact code. main.ts exposes it on IPC handlers
 * that answer only the app's own windows; there is no daemon route, workspace frame or tool for it,
 * so an agent can neither start nor answer an import.
 */

export type CookieJarWithRemoval = {
  set(details: ElectronCookieDetails): Promise<void>;
  remove(url: string, name: string): Promise<void>;
};

export type ChromeImportServiceDeps = {
  platform: Platform;
  /** Chrome's user data folder, or `null` where none is known. */
  dataRoot: string | null;
  /** The browser's own state folder (manifest, store); `null` keeps everything in memory. */
  stateDir: string | null;
  cookies: CookieJarWithRemoval;
  store: BrowserStore;
  preferences: BrowserPreferencesStore;
  /** Called after the settings change, so main can apply the languages to the session. */
  onPreferencesChanged?: (preferences: BrowserPreferences) => void;
  keyProvider: KeyProvider;
  log: ImportLog;
  now?: () => number;
  tempRoot?: string;
};

export type ChromeImportService = ReturnType<typeof createChromeImportService>;

const MAX_SITES = 10_000;

export function validImportRequest(value: unknown): ChromeImportRequest | null {
  if (!value || typeof value !== 'object') return null;
  const request = value as Record<string, unknown>;
  if (typeof request.profileId !== 'string' || request.profileId.length > 64) return null;
  for (const key of ['logins', 'bookmarks', 'history', 'settings'] as const) {
    if (typeof request[key] !== 'boolean') return null;
  }
  let sites: string[] | null = null;
  if (request.sites !== null) {
    if (!Array.isArray(request.sites) || request.sites.length > MAX_SITES) return null;
    if (!request.sites.every((site) => typeof site === 'string' && site.length <= 253)) return null;
    sites = request.sites as string[];
  }
  return {
    profileId: request.profileId,
    logins: request.logins as boolean,
    bookmarks: request.bookmarks as boolean,
    history: request.history as boolean,
    settings: request.settings as boolean,
    sites,
  };
}

export function createChromeImportService(deps: ChromeImportServiceDeps) {
  const now = deps.now ?? (() => Date.now());
  const tempRoot = deps.tempRoot ?? os.tmpdir();
  const loginsSupported = deps.platform === 'darwin' || deps.platform === 'linux';
  let running = false;

  const profiles = (): ChromeImportProfile[] =>
    deps.dataRoot ? readChromeProfiles(deps.dataRoot) : [];

  const applySettings =
    (record: { previousSettings: BrowserPreferences }) =>
    (chrome: ChromePreferences): AppliedSettings => {
      const before = deps.preferences.get();
      const next: BrowserPreferences = { ...before };
      const applied: AppliedSettings = [];
      const take = <K extends keyof BrowserPreferences>(
        label: AppliedSettings[number],
        key: K,
        value: BrowserPreferences[K] | null
      ) => {
        if (value === null || value === undefined) return;
        if (!(key in record.previousSettings)) {
          record.previousSettings = { ...record.previousSettings, [key]: before[key] ?? null };
        }
        next[key] = value;
        applied.push(label);
      };
      take('search engine', 'searchEngine', chrome.searchEngine);
      take('languages', 'acceptLanguages', chrome.acceptLanguages);
      take('default zoom', 'defaultZoomFactor', chrome.defaultZoomFactor);
      if (applied.length > 0) {
        const saved = deps.preferences.set(next);
        deps.onPreferencesChanged?.(saved);
      }
      return applied;
    };

  return {
    status(): ChromeImportStatus {
      const manifest = readManifest(deps.stateDir);
      const available = profiles().length > 0;
      return {
        available,
        offerFirstUse: available && !manifest.offerDismissed && manifest.imports.length === 0,
        loginsSupported,
        asksForKeyAccess: deps.platform === 'darwin',
        imports: manifest.imports.map(summarize),
      };
    },

    profiles,

    sites(profileId: string): ChromeImportSites {
      const dir = deps.dataRoot ? chromeProfileDir(deps.dataRoot, profileId) : null;
      if (!dir || !loginsSupported) return { readable: false, sites: [], total: 0 };
      return planCookieSites(dir, tempRoot);
    },

    dismissOffer(): void {
      const manifest = readManifest(deps.stateDir);
      manifest.offerDismissed = true;
      writeManifest(deps.stateDir, manifest);
    },

    /**
     * Import what `request` names, once `confirm` (main's own native dialog, S3) says the person
     * chose to. Nothing in the profile but its name is read before that answer.
     */
    async run(request: ChromeImportRequest, confirm: ConfirmImport): Promise<ChromeImportResult> {
      if (running) return { ok: false, error: 'An import is already running.' };
      const profile = profiles().find((candidate) => candidate.id === request.profileId);
      const dir =
        profile && deps.dataRoot ? chromeProfileDir(deps.dataRoot, request.profileId) : null;
      if (!profile || !dir) return { ok: false, error: 'That Chrome profile was not found.' };
      if (!request.logins && !request.bookmarks && !request.history && !request.settings) {
        return { ok: false, error: 'Choose something to import.' };
      }
      running = true;
      let confirmed = false;
      try {
        confirmed = await confirm(importConfirmation(request, profile.name, deps.platform));
      } catch {
        confirmed = false;
      }
      if (!confirmed) {
        running = false;
        deps.log('cancelled', {});
        return { ok: false, error: 'The import was cancelled.', cancelled: true };
      }
      try {
        const manifest = readManifest(deps.stateDir);
        const previous = manifest.imports.find(
          (record) => record.source === 'chrome' && record.profileId === profile.id
        );
        const id = `chrome-${crypto.randomBytes(8).toString('hex')}`;
        const label = `Imported from Chrome (${profile.name})`;
        const settingsRecord = { previousSettings: { ...(previous?.previousSettings ?? {}) } };
        const report = await runChromeImport({
          profileDir: dir,
          choices: {
            logins: request.logins,
            bookmarks: request.bookmarks,
            history: request.history,
            settings: request.settings,
          },
          sites: request.sites ? new Set(request.sites) : null,
          keyProvider: deps.keyProvider,
          loginsSupported,
          nowMs: now(),
          tempRoot,
          log: deps.log,
          targets: {
            cookies: deps.cookies,
            addBookmarks: (items) => deps.store.importBookmarks(id, label, items),
            addHistory: (items) =>
              deps.store.importHistory(
                id,
                label,
                items.map((item) => ({
                  url: item.url,
                  title: item.title,
                  visits: item.visits,
                  lastVisit: item.lastVisit,
                }))
              ),
            applySettings: applySettings(settingsRecord),
          },
        });

        // A second import of the same profile replaces the first: one line per profile, and
        // removal still finds every cookie either of them set. What this one did not choose is
        // carried over from the earlier one.
        const keys = new Map<string, ImportedCookieKey>();
        for (const key of [...(previous?.cookieKeys ?? []), ...report.cookieKeys]) {
          keys.set(`${key.url}\u0000${key.name}\u0000${key.domain ?? ''}`, key);
        }
        let bookmarks = report.bookmarks;
        let history = report.history;
        if (previous) {
          const earlier = deps.store.importedData(previous.id);
          deps.store.removeImport(previous.id);
          if (!request.bookmarks && earlier.bookmarks.length > 0) {
            bookmarks = deps.store.importBookmarks(id, label, earlier.bookmarks);
          } else if (!request.bookmarks) {
            bookmarks = previous.bookmarks;
          }
          if (!request.history && earlier.history.length > 0) {
            history = deps.store.importHistory(id, label, earlier.history);
          } else if (!request.history) {
            history = previous.history;
          }
        }
        const sum = (counts: Record<string, number>) =>
          Object.values(counts).reduce((total, value) => total + value, 0);
        const cookieKeys = [...keys.values()];
        const record: ImportRecord = {
          id,
          source: 'chrome',
          profileId: profile.id,
          profileName: profile.name,
          importedAt: now(),
          logins: request.logins ? report.logins : (previous?.logins ?? 'not-chosen'),
          loginsMissingKey: request.logins
            ? report.loginsMissingKey
            : (previous?.loginsMissingKey ?? null),
          cookies: cookieKeys.length,
          cookieSites: new Set(cookieKeys.map((key) => cookieSite(new URL(key.url).hostname))).size,
          skipped: request.logins ? sum(report.cookies.skipped) : (previous?.skipped ?? 0),
          failed: request.logins ? sum(report.cookies.failedBySite) : (previous?.failed ?? 0),
          bookmarks,
          history,
          settings: request.settings ? report.settings : (previous?.settings ?? null),
          cookieKeys,
          previousSettings: settingsRecord.previousSettings,
        };
        manifest.imports = [
          ...manifest.imports.filter((existing) => existing.id !== previous?.id),
          record,
        ];
        // The person has answered the first use card by importing; removing the import later
        // must not bring the card back.
        manifest.offerDismissed = true;
        writeManifest(deps.stateDir, manifest);
        return { ok: true, summary: summarize(record) };
      } catch (error) {
        deps.log('failed', { error: error instanceof Error ? error.name : 'unknown' });
        return { ok: false, error: 'The import stopped because of an unexpected error.' };
      } finally {
        running = false;
      }
    },

    /** **Remove imported data** (BB-25): cookies by key, the folder, the history, the settings. */
    async remove(importId: string): Promise<boolean> {
      // An import in flight is still writing the manifest this would read.
      if (running) return false;
      const manifest = readManifest(deps.stateDir);
      const record = manifest.imports.find((candidate) => candidate.id === importId);
      if (!record) return false;
      let removedCookies = 0;
      for (const key of record.cookieKeys) {
        try {
          await deps.cookies.remove(key.url, key.name);
          removedCookies += 1;
        } catch {
          // Already gone, or refused: removal goes on with the rest.
        }
      }
      const removed = deps.store.removeImport(record.id);
      const restore = Object.keys(record.previousSettings) as Array<keyof BrowserPreferences>;
      if (restore.length > 0) {
        const next: BrowserPreferences = { ...deps.preferences.get() };
        for (const key of restore) {
          const value = record.previousSettings[key];
          if (value === null || value === undefined) delete next[key];
          else (next as Record<string, unknown>)[key] = value;
        }
        const saved = deps.preferences.set(next);
        deps.onPreferencesChanged?.(saved);
      }
      manifest.imports = manifest.imports.filter((candidate) => candidate.id !== record.id);
      writeManifest(deps.stateDir, manifest);
      deps.log('removed', {
        cookies: removedCookies,
        bookmarks: removed.bookmarks,
        history: removed.history,
        settings: restore.length,
      });
      return true;
    },
  };
}
