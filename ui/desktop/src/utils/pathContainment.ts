import fsSync from 'node:fs';
import os from 'node:os';
import path from 'node:path';

/**
 * Pure containment logic behind the main process's file-access allowlist
 * (`isAllowedFilePath` in main.ts). Split out so it can be unit-tested without
 * importing the Electron main module.
 *
 * Canonicalization matters in BOTH directions. macOS's `/tmp` → `/private/tmp`
 * and `/var` → `/private/var` symlinks defeat naive prefix matching: `/tmp/x`
 * never string-matches a `/private/tmp` root (false DENIAL — observed live as
 * "Access denied: path '/tmp/qa-r1b/hi.txt' is outside allowed directories"
 * for a file the session's own tool call had just written), and a symlink
 * planted inside an allowed root can point outside it (false ADMIT).
 */
export function canonicalizeForContainment(p: string): string {
  try {
    return fsSync.realpathSync(p);
  } catch {
    // Path may not exist yet (e.g. a write target): canonicalize the deepest
    // existing ancestor and re-append the remainder lexically.
    const parent = path.dirname(p);
    if (parent === p) return p;
    return path.join(canonicalizeForContainment(parent), path.basename(p));
  }
}

/** True iff `candidate` (already path.resolve'd) lives under one of `roots`. */
export function isPathContained(candidate: string, roots: string[]): boolean {
  const canonical = canonicalizeForContainment(candidate);
  return roots.some((root) => {
    const canonicalRoot = canonicalizeForContainment(root);
    return canonical.startsWith(canonicalRoot + path.sep) || canonical === canonicalRoot;
  });
}

// --- Directive 2: sensitive-path denial + mode-aware preview scope ----------
//
// In Fully-Automatic mode the backend lets the agent create files anywhere, so
// the preview panel must be able to READ what it created anywhere — otherwise a
// file outside home/tmp shows "Access denied" for a file the session itself
// wrote. `isFilePathAllowedForPreview` widens the allowlist to the whole
// filesystem ONLY in Auto mode (`fullyAutomatic: true`, read from the config
// file in the MAIN process — never from a renderer IPC message). Regardless of
// mode, the small set of extremely-sensitive locations below stays denied for
// preview, mirroring the backend `security::sensitive_ops` list.

/** Absolute, `/`-separated, lowercased prefixes whose contents are sensitive.
 *  Deliberately excludes `/var` and `/tmp`: OS temp trees live there and must
 *  stay previewable (see `isTempPreviewPath`). Both the plain and macOS
 *  symlink-resolved forms of `/etc` are listed. */
const SENSITIVE_ABSOLUTE_PREFIXES = [
  '/system',
  '/usr',
  '/bin',
  '/sbin',
  '/etc',
  '/private/etc',
  '/library', // the SYSTEM /Library — a user's ~/Library is handled per-subpath
  '/boot',
  '/lib',
  '/lib64',
];

function comparisonPath(p: string): string {
  return canonicalizeForContainment(p).toLowerCase().split(path.sep).join('/');
}

function windowsSensitivePrefixes(): string[] {
  if (process.platform !== 'win32') return [];

  const systemDrive = process.env.SystemDrive || path.parse(process.cwd()).root.slice(0, 2) || 'C:';
  const roots = [
    process.env.SystemRoot,
    process.env.windir,
    process.env.ProgramFiles,
    process.env['ProgramFiles(x86)'],
    process.env.ProgramData,
    `${systemDrive}\\Windows`,
    `${systemDrive}\\Program Files`,
    `${systemDrive}\\Program Files (x86)`,
    `${systemDrive}\\ProgramData`,
  ];
  return [...new Set(roots.filter((root): root is string => !!root).map(comparisonPath))];
}

/** Credential / persistence directories under $HOME, `/`-separated + lowercased
 *  relative to the canonical home dir. Mirrors the backend list. */
const SENSITIVE_HOME_SUBPATHS = [
  '.ssh',
  '.gnupg',
  '.aws',
  '.config/gcloud',
  '.kube',
  '.docker',
  'library/keychains',
  'library/launchagents',
  'library/application support/google/chrome',
  'library/application support/chromium',
  'library/application support/bravesoftware',
  'library/application support/firefox',
  'library/application support/microsoft edge',
  '.config/google-chrome',
  '.config/chromium',
  '.config/bravesoftware',
  '.mozilla',
];

/** True for an OS temp tree (`/tmp`, `$TMPDIR`, `/var/folders/**`), which is
 *  ordinary scratch space and must never be treated as sensitive. */
function isTempPreviewPath(canonicalLower: string): boolean {
  const tempRoots = [comparisonPath('/tmp'), comparisonPath(os.tmpdir())];
  if (tempRoots.some((r) => canonicalLower === r || canonicalLower.startsWith(r + '/'))) {
    return true;
  }
  return (
    canonicalLower.startsWith('/var/folders/') || canonicalLower.startsWith('/private/var/folders/')
  );
}

/** `p` resolved and lowercased with `/` separators, but NOT canonicalized: the
 *  spelling the caller used, so a home named through a link still matches. */
function spelledPath(p: string): string {
  return path.resolve(p).toLowerCase().split(path.sep).join('/');
}

/** Whether `candidate` (in comparison form) lies in one of the credential and
 *  persistence folders below `home` (in the same form). */
function isSensitiveHomeSubpath(candidate: string, home: string): boolean {
  if (!home || home === '/' || !(candidate === home || candidate.startsWith(home + '/'))) {
    return false;
  }
  const rel = candidate === home ? '' : candidate.slice(home.length + 1);
  return SENSITIVE_HOME_SUBPATHS.some((sub) => rel === sub || rel.startsWith(sub + '/'));
}

/**
 * Whether previewing `candidate` would expose an extremely-sensitive location
 * (system directory, SSH keys, keychains, launchd, cloud creds, browser
 * credential stores). Denied in EVERY mode. Symlink-aware: the path is
 * canonicalized first so a symlink cannot dodge the check.
 *
 * ⚠ The home-folder deny runs BEFORE the temp-tree exemption, and is asked of
 * the canonical home and of HOME as given. The exemption used to come first,
 * so with HOME under `/tmp`, `$TMPDIR` or `/var/folders` (a test profile, a
 * container, a CI runner) every `~/.ssh`, `~/.aws` and `~/Library/Keychains`
 * path was "temp scratch" and previewable in every mode (AG-F6). A credential
 * folder is one wherever the home happens to live.
 */
export function isSensitivePreviewPath(candidate: string): boolean {
  const canonical = comparisonPath(candidate);
  const spelled = spelledPath(candidate);
  const homeGiven = os.homedir();
  if (homeGiven) {
    const homes = [comparisonPath(homeGiven), spelledPath(homeGiven)];
    for (const home of homes) {
      if (isSensitiveHomeSubpath(canonical, home) || isSensitiveHomeSubpath(spelled, home)) {
        return true;
      }
    }
  }

  if (isTempPreviewPath(canonical)) return false;

  for (const prefix of [...SENSITIVE_ABSOLUTE_PREFIXES, ...windowsSensitivePrefixes()]) {
    if (canonical === prefix || canonical.startsWith(prefix + '/')) return true;
  }
  return false;
}

/**
 * The preview allowlist decision, factoring in the permission mode.
 *   - A sensitive path (above) is denied in EVERY mode.
 *   - In Fully-Automatic mode any other path is admitted (parity with the
 *     backend, which lets the agent write anywhere).
 *   - Otherwise the path must be contained by one of `roots` (home / temp / …).
 */
export function isFilePathAllowedForPreview(
  candidate: string,
  roots: string[],
  opts: { fullyAutomatic: boolean }
): boolean {
  if (isSensitivePreviewPath(candidate)) return false;
  if (opts.fullyAutomatic) return true;
  return isPathContained(candidate, roots);
}

/**
 * The preview allowlist's ROOTS, as a pure function of its inputs.
 *
 * Split out of `main.ts`'s `allowedFileRoots` so the set can be asserted at all:
 * nothing can import `main.ts` under vitest, and a test that hand-builds a roots
 * array proves only that `isPathContained` works — it cannot catch a root that
 * was never added, which is precisely the defect this exists to prevent.
 */
export function previewFileRoots(opts: {
  /** The folder the user pointed THIS chat at, when there is one. */
  sessionWorkingDir?: string;
  home: string;
  userData: string;
  appTemp: string;
  systemTemp: string;
  platform: typeof process.platform;
  pathRootOverride?: string;
}): string[] {
  return [
    // ⚠ First, and load-bearing. Without it a working directory outside the
    // home tree made every file the agent had just written unpreviewable: the
    // panel refused a file the session itself created, while the session kept
    // writing there. Choosing the folder is the act that puts it in scope.
    ...(opts.sessionWorkingDir ? [opts.sessionWorkingDir] : []),
    opts.home,
    opts.userData,
    opts.appTemp,
    opts.systemTemp,
    ...(opts.platform !== 'win32' ? ['/tmp'] : []),
    ...(opts.pathRootOverride ? [opts.pathRootOverride] : []),
  ];
}
