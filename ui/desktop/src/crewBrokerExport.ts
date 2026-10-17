import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';

/**
 * "Save Crew's server program…" (OW-9): the manual install's copy of the Linux broker this desktop
 * build carries, `crew-broker/linux-x86_64/` beside `biorouterd` with the manifest staging wrote.
 *
 * Main-process only. The renderer names the version and SHA-256 it showed the person (the daemon's
 * `GET /crew/setup/artifact`); nothing is written unless the bundled file's own bytes hash to that
 * digest and its manifest says the same, so the commands on screen always check the file saved.
 */

/** The largest broker this reads, the daemon's own bound on the artifact. */
export const MAX_BROKER_BYTES = 64 * 1024 * 1024;

const VERSION = /^\d+\.\d+\.\d+$/;
const SHA256 = /^[0-9a-f]{64}$/;

export interface SaveCrewBrokerDeps {
  /** Where the bundle is, or null when this build carries none. */
  brokerDirectory(): string | null;
  readFile(file: string): Promise<Buffer>;
  writeFile(file: string, data: Buffer): Promise<void>;
  /** Where the save dialog starts. */
  defaultFolder(): string;
  showSaveDialog(options: {
    title: string;
    defaultPath: string;
    buttonLabel: string;
  }): Promise<{ canceled: boolean; filePath?: string }>;
}

export type SaveCrewBrokerResult =
  | { success: true; filePath: string }
  | { success: false; canceled: true }
  | { success: false; error: string };

/**
 * The bundle's folder, as the daemon looks for it: `BIOROUTER_CREW_BROKER_DIR` when set (development
 * and tests), else beside the `biorouterd` this app starts. A candidate that does not hold the
 * program is skipped; null when none does.
 */
export function crewBrokerDirectory(
  app: { isPackaged: boolean },
  env: Record<string, string | undefined> = process.env,
  exists: (file: string) => boolean = defaultExists,
  resourcesPath: string = process.resourcesPath,
  cwd: string = process.cwd()
): string | null {
  const override = env.BIOROUTER_CREW_BROKER_DIR?.trim();
  if (override) return exists(path.join(override, 'biorouter-crew')) ? override : null;
  const relative = path.join('crew-broker', 'linux-x86_64');
  const candidates = app.isPackaged
    ? [path.join(resourcesPath, 'bin', relative)]
    : [
        path.join(cwd, '..', '..', 'target', 'debug', relative),
        path.join(cwd, '..', '..', 'target', 'release', relative),
        path.join(cwd, 'src', 'bin', relative),
      ];
  return candidates.find((dir) => exists(path.join(dir, 'biorouter-crew'))) ?? null;
}

function defaultExists(file: string): boolean {
  try {
    return fs.statSync(file).isFile();
  } catch {
    return false;
  }
}

function expectedFrom(value: unknown): { version: string; sha256: string } | null {
  if (typeof value !== 'object' || value === null) return null;
  const { version, sha256 } = value as { version?: unknown; sha256?: unknown };
  if (typeof version !== 'string' || !VERSION.test(version)) return null;
  if (typeof sha256 !== 'string' || !SHA256.test(sha256)) return null;
  return { version, sha256 };
}

/** Check the bundle against what the person was shown, then save a copy where they choose. */
export async function saveCrewBroker(
  expected: unknown,
  deps: SaveCrewBrokerDeps
): Promise<SaveCrewBrokerResult> {
  const want = expectedFrom(expected);
  if (!want) return { success: false, error: 'Invalid Crew server program request.' };
  const dir = deps.brokerDirectory();
  if (!dir)
    return { success: false, error: 'This Biorouter doesn’t include Crew’s server program.' };
  let bytes: Buffer;
  try {
    bytes = await deps.readFile(path.join(dir, 'biorouter-crew'));
  } catch {
    return { success: false, error: 'Crew’s server program couldn’t be read.' };
  }
  // A release bundle always has its manifest; a development copy (`BIOROUTER_CREW_BROKER_DIR`)
  // may not, as the daemon allows. Then the bytes' own digest is the whole check.
  let manifest: { version?: unknown; sha256?: unknown; size?: unknown } | null = null;
  try {
    manifest = JSON.parse((await deps.readFile(path.join(dir, 'manifest.json'))).toString('utf8'));
  } catch {
    manifest = null;
  }
  const digest = createHash('sha256').update(bytes).digest('hex');
  if (
    bytes.length === 0 ||
    bytes.length > MAX_BROKER_BYTES ||
    digest !== want.sha256 ||
    (manifest !== null &&
      (typeof manifest !== 'object' ||
        manifest.sha256 !== want.sha256 ||
        manifest.version !== want.version ||
        (manifest.size !== undefined && manifest.size !== bytes.length)))
  ) {
    return {
      success: false,
      error: 'Crew’s server program doesn’t match the one shown, so it wasn’t saved.',
    };
  }
  const name = `biorouter-crew-${want.version}-linux-x86_64`;
  const choice = await deps.showSaveDialog({
    title: 'Save Crew’s server program',
    defaultPath: path.join(deps.defaultFolder(), name),
    buttonLabel: 'Save',
  });
  if (choice.canceled || !choice.filePath) return { success: false, canceled: true };
  try {
    await deps.writeFile(choice.filePath, bytes);
  } catch {
    return { success: false, error: 'Biorouter couldn’t save the program there.' };
  }
  return { success: true, filePath: choice.filePath };
}
