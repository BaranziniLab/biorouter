import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';

/**
 * Read one of Chrome's SQLite files without touching it (BB-22, "Never touch the live files").
 *
 * Chrome holds its databases open while it runs, on some systems with an exclusive lock, and a
 * reader that opened the live file could block Chrome or be blocked by it. So the file is copied,
 * with its `-wal` and `-journal` companions, into a private temporary folder (mode 0700), the copy
 * is opened, read and closed, and the folder is deleted, whatever happened. The last few seconds of
 * Chrome's writes may be missing from the copy; that is acceptable.
 *
 * The copy is opened read and write: a copy with a `-wal` file can only be read by a connection
 * that may recover the log into it. It is this function's own throwaway file.
 */

export class UnreadableDatabaseError extends Error {
  constructor(readonly file: string) {
    super(`${path.basename(file)} could not be read`);
  }
}

const COMPANIONS = ['-wal', '-journal'];
/** A cookie or history database larger than this is not a Chrome profile's. */
export const MAX_DATABASE_BYTES = 512 * 1024 * 1024;

export function withCopiedDatabase<T>(
  file: string,
  read: (db: DatabaseSync) => T,
  tempRoot: string = os.tmpdir()
): T {
  let stat: fs.Stats;
  try {
    stat = fs.statSync(file);
  } catch {
    throw new UnreadableDatabaseError(file);
  }
  if (!stat.isFile() || stat.size > MAX_DATABASE_BYTES) throw new UnreadableDatabaseError(file);

  const dir = fs.mkdtempSync(path.join(tempRoot, 'biorouter-chrome-import-'));
  try {
    fs.chmodSync(dir, 0o700);
    const copy = path.join(dir, `db-${crypto.randomBytes(6).toString('hex')}`);
    try {
      fs.copyFileSync(file, copy);
      fs.chmodSync(copy, 0o600);
    } catch {
      throw new UnreadableDatabaseError(file);
    }
    for (const suffix of COMPANIONS) {
      try {
        fs.copyFileSync(`${file}${suffix}`, `${copy}${suffix}`);
      } catch {
        // No companion, or one Chrome is rewriting: the main file alone is still a database.
      }
    }
    let db: DatabaseSync;
    try {
      db = new DatabaseSync(copy);
    } catch {
      throw new UnreadableDatabaseError(file);
    }
    try {
      return read(db);
    } catch (error) {
      if (error instanceof UnreadableDatabaseError) throw error;
      throw new UnreadableDatabaseError(file);
    } finally {
      db.close();
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

/** The columns a table has, lower cased; an empty set when the table does not exist. */
export function tableColumns(db: DatabaseSync, table: string): Set<string> {
  if (!/^[a-z_]+$/.test(table)) return new Set();
  const rows = db.prepare(`PRAGMA table_info(${table})`).all() as Array<{ name?: unknown }>;
  return new Set(
    rows.map((row) => String(row.name ?? '').toLowerCase()).filter((name) => name.length > 0)
  );
}

/** `meta.version` of a Chrome database, or 0 when it has none. */
export function metaVersion(db: DatabaseSync): number {
  if (!tableColumns(db, 'meta').has('key')) return 0;
  const row = db.prepare("SELECT value FROM meta WHERE key = 'version'").get() as
    | { value?: unknown }
    | undefined;
  const version = Number.parseInt(String(row?.value ?? ''), 10);
  return Number.isFinite(version) ? version : 0;
}
