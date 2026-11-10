import { isCrewTask } from '../components/chats/chatKind';
import { isBrowserSurface } from './surface';

/**
 * How the interface writes a folder for a person to read: one rule, so the sidebar, the composer
 * footer's folder chip and its card, History and the terminal dock never write the same folder two
 * ways. Renderer-safe (no node modules), which is why it is not `utils/pathUtils.ts`.
 *
 * - The whole location is `tildePath(path, homeFolder())`: `~` for the home folder,
 *   `~/Desktop/BioRouter` under it, and absolute anywhere else, separators as the daemon wrote
 *   them (`~\Desktop\x` on Windows).
 * - A folder's short name is `folderName(path, homeFolder())`, the word the sidebar's folder groups
 *   use (`folderLabels` in BioRouterSidebar/sidebarChatView.ts): its basename, `~` for home and
 *   `Crew` for the Crew task folder.
 * - `folderSegments` splits a written path into its names and separators, so a piece of interface
 *   can isolate each name (`<bdi>`) and keep a right-to-left name from reordering the separators.
 *
 * `normalizeFolder` and `tildePath` moved here from sidebarChatView.ts, which re-exports them.
 */

/** Trailing separators off, except on a root (`/`, `C:\`). */
export function normalizeFolder(path: string | null | undefined): string {
  const trimmed = (path ?? '').trim();
  if (!trimmed) return '';
  const stripped = trimmed.replace(/[\\/]+$/, '');
  return stripped || trimmed.slice(0, 1);
}

/** The path with the home folder written as `~`. */
export function tildePath(path: string, homeDir?: string | null): string {
  const home = normalizeFolder(homeDir);
  if (!home || !path) return path;
  if (path === home) return '~';
  if (path.startsWith(`${home}/`) || path.startsWith(`${home}\\`)) {
    return `~${path.slice(home.length)}`;
  }
  return path;
}

/**
 * The home folder a path the daemon reports may be written against, or `null` when that would
 * mislead. It is this machine's home folder (`BIOROUTER_HOME_DIR`, which the main process puts in
 * every window's config, display only), and only while the daemon is this machine's: the desktop's
 * own managed daemon (`BIOROUTER_LOCAL_BACKEND`, the test `folderAccessLifetime()` makes). A
 * `biorouter serve` browser, or a desktop pointed at an external backend, may be reading folders on
 * another machine, where `~` would name the wrong home, so a path stays absolute there.
 */
export function homeFolder(): string | null {
  if (typeof window === 'undefined' || isBrowserSurface()) return null;
  const config = window.appConfig;
  if (config?.get('BIOROUTER_LOCAL_BACKEND') !== true) return null;
  const home = config.get('BIOROUTER_HOME_DIR');
  return typeof home === 'string' && home.trim() ? home : null;
}

/** One piece of a written path: a folder's name, or the separator run between two names. */
export interface FolderSegment {
  text: string;
  separator: boolean;
}

/**
 * The written path (`tildePath`'s answer) in order, names and separators apart, every character
 * kept: joining the pieces gives the path back exactly, so a selection of the drawn path copies
 * what it says. A run of separators (`\\server`) stays one piece.
 */
export function folderSegments(path: string): FolderSegment[] {
  return path
    .split(/([\\/]+)/)
    .filter((text) => text !== '')
    .map((text) => ({ text, separator: /^[\\/]+$/.test(text) }));
}

/** A Windows drive root ("C:\", "C:/"): a root, not a folder named "C:". */
const DRIVE_ROOT = /^[A-Za-z]:[\\/]+$/;

/**
 * What the sidebar's folder groups call the Crew task folder (`CREW_FOLDER_LABEL`). Every Crew task
 * runs in `<data dir>/crew/tasks`, whose basename says nothing. folderPath.test.ts holds the two
 * equal.
 */
const CREW_FOLDER_NAME = 'Crew';

/**
 * A folder's short name, as the sidebar's folder groups write it: the basename ("Desktop" for
 * `/Users/me/Desktop`, trailing separators ignored), `~` for the home folder, `Crew` for the Crew
 * task folder. A root has no basename and is shown as it is written (`/`, `C:\`). Unlike the
 * sidebar, which keys its groups on `normalizeFolder` and so shows a drive root as `C:`, the root
 * keeps its separator here.
 */
export function folderName(path: string, homeDir?: string | null): string {
  if (DRIVE_ROOT.test(path)) return path;
  const folder = normalizeFolder(path);
  if (!folder) return path;
  const home = normalizeFolder(homeDir);
  if (home && folder === home) return '~';
  if (isCrewTask({ working_dir: folder })) return CREW_FOLDER_NAME;
  const names = folderSegments(folder).filter((segment) => !segment.separator);
  return names.length > 0 ? names[names.length - 1].text : folder;
}
