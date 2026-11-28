import AdmZip from 'adm-zip';
import fs from 'node:fs';
import path from 'node:path';

function safeEntryName(entryName: string): string {
  if (!entryName || entryName.includes('\0')) {
    throw new Error(`Unsafe zip entry path: ${entryName}`);
  }
  if (path.isAbsolute(entryName) || path.win32.isAbsolute(entryName)) {
    throw new Error(`Unsafe zip entry path: ${entryName}`);
  }

  // A `.` segment names the folder it sits in, so it is dropped here: two
  // entries that differ only by one (`manifest.json` and `./manifest.json`)
  // name one file on every disk, and the checks below must see one name.
  const parts = entryName.split(/[\\/]+/).filter((part) => part && part !== '.');
  if (parts.some((part) => part === '..')) {
    throw new Error(`Unsafe zip entry path: ${entryName}`);
  }
  return parts.join(path.sep);
}

export function safeZipEntryTarget(baseDir: string, entryName: string): string {
  const relativeName = safeEntryName(entryName);
  const base = path.resolve(baseDir);
  const resolved = path.resolve(base, relativeName);
  if (resolved !== base && !resolved.startsWith(base + path.sep)) {
    throw new Error(`Unsafe zip entry path: ${entryName}`);
  }
  return resolved;
}

function checkExistingTarget(base: string, target: string): void {
  const physicalBase = fs.realpathSync(base);
  let physicalTarget = physicalBase;
  // Resolve existing components before creating anything. An existing link
  // whose destination cannot be resolved must not fall back to a lexical path.
  for (const part of path.relative(base, target).split(path.sep).filter(Boolean)) {
    physicalTarget = path.join(physicalTarget, part);
    if (fs.lstatSync(physicalTarget, { throwIfNoEntry: false })) {
      physicalTarget = fs.realpathSync(physicalTarget);
    }
    if (physicalTarget !== physicalBase && !physicalTarget.startsWith(physicalBase + path.sep)) {
      throw new Error(`Unsafe zip entry path: ${target}`);
    }
  }
}

/**
 * One path part as the most forgiving disk Biorouter writes to reads it. It
 * folds MORE than any one filesystem does, which only refuses more archives:
 *
 * - Unicode compatibility forms (NFKC), which covers the canonical forms APFS
 *   and NTFS treat as one name;
 * - case, by Unicode rules rather than `toLowerCase` alone: upper then lower
 *   case, so U+017F (long s) is `s`, as a case-insensitive APFS reads it
 *   (measured), and `ß` is `ss`, which only over-folds;
 * - trailing dots and spaces, which Windows drops (`manifest.json.` is
 *   `manifest.json` there).
 */
function foldedPart(part: string): string {
  const folded = part.normalize('NFKC').toUpperCase().toLowerCase().normalize('NFKC');
  return folded.replace(/[. ]+$/, '') || folded;
}

/** An entry's path parts, each folded by {@link foldedPart}. */
function foldedParts(entryName: string): string[] {
  return safeEntryName(entryName).split(path.sep).filter(Boolean).map(foldedPart);
}

/** An 8.3 short name (`MANIFE~1.JSO`), which Windows can resolve to a long name. */
const SHORT_NAME = /^[^.]{1,8}(\.[^.]{0,3})?$/;

/**
 * Why an entry's name may not be extracted at all, whatever else the archive
 * holds, or `undefined`. Both shapes reach a file no rule over the long name
 * sees: NTFS reads `manifest.json::$DATA` as `manifest.json` (a colon names a
 * stream), and can read `MANIFE~1.JSO` as it too. A colon is not a legal name
 * character on Windows anyway.
 */
function unsafeNameReason(entryName: string): string | undefined {
  for (const part of safeEntryName(entryName).split(path.sep)) {
    if (part.includes(':')) return 'a colon, which names a file stream on Windows';
    if (/~\d/.test(part) && SHORT_NAME.test(part)) {
      return 'the shape of an 8.3 short name, which Windows can read as another file';
    }
  }
  return undefined;
}

/**
 * Refuse an archive in which two entries could name one file on a disk Biorouter
 * writes to: the same name twice; two names that differ only in case (by Unicode
 * rules), Unicode form, a `.` segment, or trailing dots and spaces; or a file
 * that is also a folder of another entry (`lib` and `lib/module.py`). Also
 * refuses a name with a colon or the shape of an 8.3 short name, which Windows
 * can resolve to another entry's file.
 *
 * ⚠ **Why.** `zip.getEntry('manifest.json')` reads ONE entry, while
 * `safeExtractZip` writes every entry in order, so a bundle carrying
 * `manifest.json` and then `./manifest.json`, `MANIFEST.JSON` or `manifeſt.json`
 * passes a check of the first and installs the second. Folders may repeat (a
 * folder entry beside the files in it is ordinary); files may not, and a file
 * may not share a folder's name. Also refuses an entry `safeZipEntryTarget`
 * would, so a caller can check a whole archive before it writes anything.
 *
 * ⚠ **A rule over names cannot know every disk's rules**, so this is the first
 * of two layers: `safeExtractZip` also refuses to write onto a file an earlier
 * entry wrote, and the extension updater reads the extracted `manifest.json`
 * back and compares it with the bytes it checked.
 */
export function assertDistinctZipEntries(zip: AdmZip): void {
  const files = new Map<string, string>();
  const folders = new Map<string, string>();
  const clash = (first: string, second: string) =>
    new Error(`Zip entries ${first} and ${second} name one file on this disk`);
  for (const entry of zip.getEntries()) {
    const unsafe = unsafeNameReason(entry.entryName);
    if (unsafe) {
      throw new Error(`Zip entry ${entry.entryName} cannot be extracted safely: it has ${unsafe}`);
    }
    const parts = foldedParts(entry.entryName);
    if (parts.length === 0) continue;
    const folded = parts.join('/');
    for (let depth = 1; depth < parts.length; depth++) {
      const folder = parts.slice(0, depth).join('/');
      const file = files.get(folder);
      if (file !== undefined) throw clash(file, entry.entryName);
      if (!folders.has(folder)) folders.set(folder, entry.entryName);
    }
    if (entry.isDirectory) {
      const file = files.get(folded);
      if (file !== undefined) throw clash(file, entry.entryName);
      if (!folders.has(folded)) folders.set(folded, entry.entryName);
      continue;
    }
    const earlier = files.get(folded) ?? folders.get(folded);
    if (earlier !== undefined) throw clash(earlier, entry.entryName);
    files.set(folded, entry.entryName);
  }
}

/**
 * The file a path reaches (through links), as its device and inode, or
 * `undefined` when there is none or the disk reports no inode (0).
 */
function fileIdentity(target: string): string | undefined {
  const stat = fs.statSync(target, { bigint: true, throwIfNoEntry: false });
  if (!stat || stat.ino === 0n) return undefined;
  return `${stat.dev}:${stat.ino}`;
}

export function safeExtractZip(zip: AdmZip, destinationDir: string): void {
  // The whole archive first: a refusal part way would leave the entries before
  // it written.
  assertDistinctZipEntries(zip);
  const base = path.resolve(destinationDir);
  fs.mkdirSync(base, { recursive: true });

  // The second layer (see `assertDistinctZipEntries`): the file each entry was
  // written to. An entry that would land on one of them, through any alias the
  // name rules do not know (an existing hard link or link inside the folder, a
  // disk's own folding), is refused before it overwrites what was written.
  const written = new Map<string, string>();
  for (const entry of zip.getEntries()) {
    const target = safeZipEntryTarget(base, entry.entryName);
    checkExistingTarget(base, target);
    if (entry.isDirectory) {
      fs.mkdirSync(target, { recursive: true });
      continue;
    }

    fs.mkdirSync(path.dirname(target), { recursive: true });
    const before = fileIdentity(target);
    const earlier = before === undefined ? undefined : written.get(before);
    if (earlier !== undefined) {
      throw new Error(`Zip entries ${earlier} and ${entry.entryName} name one file on this disk`);
    }
    fs.writeFileSync(target, entry.getData());
    const after = fileIdentity(target);
    if (after !== undefined) written.set(after, entry.entryName);
  }
}

/**
 * Whether the file an extraction left at `entryName` holds exactly the bytes of
 * the archive's own `entryName` entry, the one `zip.getEntry` returns and a
 * caller checked. False when either is missing.
 *
 * The check that mirrors no disk's rules: whatever aliasing the layers above
 * missed, a checked file that is not on disk as checked is refused.
 */
export function extractedFileMatchesEntry(
  zip: AdmZip,
  destinationDir: string,
  entryName: string
): boolean {
  const entry = zip.getEntry(entryName);
  if (!entry || entry.isDirectory) return false;
  let onDisk: Buffer;
  try {
    onDisk = fs.readFileSync(safeZipEntryTarget(destinationDir, entryName));
  } catch {
    return false;
  }
  return onDisk.equals(entry.getData());
}
