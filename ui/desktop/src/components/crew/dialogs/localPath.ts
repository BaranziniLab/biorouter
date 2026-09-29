/**
 * Paths on THIS computer, judged the way the daemon judges them (W2-UIW-13).
 *
 * The daemon runs on the same computer as this window: Crew works only in the desktop app
 * (`CrewNeedsDesktop`), which starts its own `biorouterd`. A connection's Identity file is checked
 * by `validate_connection` in `crates/biorouter/src/crew/mod.rs` with Rust's `Path::is_absolute`
 * for that computer's OS, and then handed to `ssh -i` unchanged. So the rule is the OS's own:
 *
 * - macOS and Linux: the path starts with `/`.
 * - Windows: a drive letter, a colon and a separator (`C:\Users\…` or `C:/Users/…`), or two
 *   separators (`\\server\share\…`, `\\?\C:\…`, `//server/share/…`). `\Users\…`, `C:Users\…` and
 *   `/Users/…` are not absolute there, and the daemon refuses them.
 * - No OS expands a leading `~`, so `~/.ssh/id_ed25519` is refused everywhere.
 *
 * When this window cannot tell its OS (a test, a detached document) either shape is accepted and
 * the daemon decides; its refusal is then said under the field.
 *
 * A remote folder is NOT judged here: it is a path on the Linux server, not on this computer.
 */

/** This computer's OS, as far as a local path's shape depends on it. */
export type LocalPlatform = 'windows' | 'mac' | 'posix' | 'unknown';

/** The OS the preload reports (`process.platform`), or `unknown` when there is no preload. */
export function localPlatform(): LocalPlatform {
  const platform =
    typeof window === 'undefined'
      ? undefined
      : (window as Window & { electron?: { platform?: unknown } }).electron?.platform;
  if (platform === 'win32') return 'windows';
  if (platform === 'darwin') return 'mac';
  return typeof platform === 'string' && platform ? 'posix' : 'unknown';
}

/** `C:\` or `C:/`: a drive letter with its root. */
const WINDOWS_DRIVE_ROOT = /^[A-Za-z]:[\\/]/;
/** `\\server\share`, `\\?\C:\`, `\\.\device`, `//server/share`: Windows reads each as rooted. */
const WINDOWS_DOUBLE_SEPARATOR = /^[\\/]{2}/;

/** Whether `path` is absolute as `Path::is_absolute` judges it on `platform`. */
export function isLocalAbsolutePath(path: string, platform: LocalPlatform): boolean {
  const windows = WINDOWS_DRIVE_ROOT.test(path) || WINDOWS_DOUBLE_SEPARATOR.test(path);
  const posix = path.startsWith('/');
  switch (platform) {
    case 'windows':
      return windows;
    case 'mac':
    case 'posix':
      return posix;
    case 'unknown':
      return windows || posix;
  }
}
