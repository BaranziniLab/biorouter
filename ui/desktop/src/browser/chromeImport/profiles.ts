import fs from 'node:fs';
import path from 'node:path';

/**
 * Where Chrome keeps its profiles, and which ones there are (BB-21).
 *
 * Pure apart from reading `Local State`, a JSON file Chrome rewrites while it runs (reading it is
 * safe: Chrome replaces it atomically). Nothing here opens a database or asks for a key.
 */

export type ChromeProfile = {
  /** The profile's folder name, `Default` or `Profile N`. */
  id: string;
  /** The name Chrome shows for it. */
  name: string;
  /** The Google account signed in to it, when there is one. Shown to the person only. */
  email: string | null;
  /** Chrome's last used profile, selected first. */
  lastUsed: boolean;
};

/** `process.platform`'s type, without the `NodeJS` global the linter does not know. */
export type Platform = typeof process.platform;

const PROFILE_FOLDER = /^(?:Default|Profile \d{1,4})$/;
const MAX_LOCAL_STATE_BYTES = 8 * 1024 * 1024;
const MAX_NAME_CHARS = 120;

/**
 * Chrome's user data folder for this system, or `null` where there is no rule for one.
 * Chromium, Brave, Edge and Arc are later additions of a path here and a key name in the helper.
 */
export function chromeDataRoot(
  platform: Platform,
  home: string,
  env: Record<string, string | undefined>
): string | null {
  if (!home) return null;
  if (platform === 'darwin') {
    return path.join(home, 'Library', 'Application Support', 'Google', 'Chrome');
  }
  if (platform === 'linux') {
    const configHome =
      env.CHROME_CONFIG_HOME ||
      (env.XDG_CONFIG_HOME && path.isAbsolute(env.XDG_CONFIG_HOME)
        ? env.XDG_CONFIG_HOME
        : path.join(home, '.config'));
    return path.join(configHome, 'google-chrome');
  }
  if (platform === 'win32') {
    return env.LOCALAPPDATA ? path.join(env.LOCALAPPDATA, 'Google', 'Chrome', 'User Data') : null;
  }
  return null;
}

function readLocalState(root: string): Record<string, unknown> | null {
  const file = path.join(root, 'Local State');
  try {
    const stat = fs.statSync(file);
    if (!stat.isFile() || stat.size > MAX_LOCAL_STATE_BYTES) return null;
    const parsed = JSON.parse(fs.readFileSync(file, 'utf8'));
    return parsed && typeof parsed === 'object' ? (parsed as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

function isDirectory(candidate: string): boolean {
  try {
    return fs.statSync(candidate).isDirectory();
  } catch {
    return false;
  }
}

function text(value: unknown, max = MAX_NAME_CHARS): string {
  return typeof value === 'string' ? value.replace(/\s+/g, ' ').trim().slice(0, max) : '';
}

/** The profiles in `root` that exist on disk, Chrome's last used one first. */
export function readChromeProfiles(root: string): ChromeProfile[] {
  if (!isDirectory(root)) return [];
  const localState = readLocalState(root);
  const profileSection =
    localState && typeof localState.profile === 'object' && localState.profile
      ? (localState.profile as Record<string, unknown>)
      : {};
  const infoCache =
    typeof profileSection.info_cache === 'object' && profileSection.info_cache
      ? (profileSection.info_cache as Record<string, unknown>)
      : {};
  const lastUsed = text(profileSection.last_used);

  const profiles: ChromeProfile[] = [];
  for (const [id, info] of Object.entries(infoCache)) {
    if (!PROFILE_FOLDER.test(id) || !isDirectory(path.join(root, id))) continue;
    const entry = info && typeof info === 'object' ? (info as Record<string, unknown>) : {};
    const email = text(entry.user_name, 320);
    profiles.push({
      id,
      name: text(entry.name) || text(entry.gaia_name) || id,
      email: email.includes('@') ? email : null,
      lastUsed: id === lastUsed,
    });
  }
  // A profile Chrome never listed (an old install, a damaged Local State) is still a profile.
  if (profiles.length === 0 && isDirectory(path.join(root, 'Default'))) {
    profiles.push({ id: 'Default', name: 'Default', email: null, lastUsed: true });
  }
  if (!profiles.some((profile) => profile.lastUsed) && profiles.length > 0) {
    const fallback = profiles.find((profile) => profile.id === 'Default') ?? profiles[0];
    fallback.lastUsed = true;
  }
  return profiles.sort(
    (a, b) =>
      Number(b.lastUsed) - Number(a.lastUsed) ||
      a.name.localeCompare(b.name) ||
      a.id.localeCompare(b.id)
  );
}

/**
 * The folder of the profile `id` under `root`, or `null` when `id` is not one of the profiles
 * `readChromeProfiles` lists. A renderer names a profile by id; main never joins an id it has not
 * just found itself, so a crafted id cannot point the reader anywhere else.
 */
export function chromeProfileDir(root: string, id: string): string | null {
  if (!PROFILE_FOLDER.test(id)) return null;
  if (!readChromeProfiles(root).some((profile) => profile.id === id)) return null;
  return path.join(root, id);
}

/** The cookie database: `Network/Cookies` since Chrome 96, `Cookies` before. */
export function chromeCookiesFile(profileDir: string): string | null {
  for (const candidate of [
    path.join(profileDir, 'Network', 'Cookies'),
    path.join(profileDir, 'Cookies'),
  ]) {
    try {
      if (fs.statSync(candidate).isFile()) return candidate;
    } catch {
      // Not this one.
    }
  }
  return null;
}

/** The files the import reads. Saved passwords (`Login Data`) and `Web Data` are never here. */
export function chromeProfileFiles(profileDir: string) {
  return {
    cookies: chromeCookiesFile(profileDir),
    bookmarks: path.join(profileDir, 'Bookmarks'),
    history: path.join(profileDir, 'History'),
    preferences: path.join(profileDir, 'Preferences'),
  };
}
