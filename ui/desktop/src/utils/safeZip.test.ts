import { describe, expect, it } from 'vitest';
import AdmZip from 'adm-zip';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {
  assertDistinctZipEntries,
  extractedFileMatchesEntry,
  safeExtractZip,
  safeZipEntryTarget,
} from './safeZip';

function tempDir(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'biorouter-safe-zip-'));
}

describe('safeExtractZip', () => {
  it('extracts regular nested files', () => {
    const dir = tempDir();
    const zip = new AdmZip();
    zip.addFile('src/main.py', Buffer.from('print("ok")'));

    safeExtractZip(zip, dir);

    expect(fs.readFileSync(path.join(dir, 'src', 'main.py'), 'utf8')).toBe('print("ok")');
  });

  it('rejects parent-directory traversal', () => {
    const dir = tempDir();
    expect(() => safeZipEntryTarget(dir, '../outside.txt')).toThrow(/Unsafe zip entry path/);
    expect(fs.existsSync(path.join(dir, '..', 'outside.txt'))).toBe(false);
  });

  it('rejects absolute paths', () => {
    const dir = tempDir();
    expect(() => safeZipEntryTarget(dir, '/tmp/outside.txt')).toThrow(/Unsafe zip entry path/);
  });

  it('rejects windows absolute paths on every platform', () => {
    const dir = tempDir();
    expect(() => safeZipEntryTarget(dir, 'C:\\Users\\Public\\outside.txt')).toThrow(
      /Unsafe zip entry path/
    );
  });
});

/**
 * Two entries a case-insensitive or normalization-insensitive disk (APFS, NTFS)
 * stores as ONE file. A reader that looks one of them up (`zip.getEntry`) and an
 * extraction that writes both in order disagree about which one is on disk, so
 * the updater's manifest check could pass a `manifest.json` that the extraction
 * then overwrote with a `MANIFEST.JSON`. Refused before anything is written.
 */
describe('entries that name one file', () => {
  function zipOf(names: string[]): AdmZip {
    const zip = new AdmZip();
    names.forEach((name, index) => {
      const placeholder = `entry-${index}.txt`;
      zip.addFile(placeholder, Buffer.from(`entry ${index}\n`));
      // adm-zip would merge two `addFile`s of one name; set the raw name, as a
      // hostile archive carries it.
      zip.getEntry(placeholder)!.entryName = name;
    });
    return zip;
  }

  it.each([
    ['the same name twice', ['manifest.json', 'manifest.json']],
    ['names that differ only in case', ['manifest.json', 'MANIFEST.JSON']],
    ['names that differ only in Unicode normalization', ['caf\u00e9.txt', 'cafe\u0301.txt']],
    ['a file and a folder of one name', ['lib', 'lib/module.py']],
    ['a file under a folder spelled two ways', ['Src/main.py', 'src/main.py']],
    // A `.` segment names the folder it sits in, on every disk.
    ['a dot segment', ['manifest.json', './manifest.json']],
    ['a doubled separator after a dot segment', ['manifest.json', './/manifest.json']],
    ['a dot segment with a backslash', ['manifest.json', '.\\manifest.json']],
    ['a dot segment inside a path', ['src/main.py', 'src/./main.py']],
    // APFS and NTFS fold case by Unicode, not by `toLowerCase`: U+017F (long
    // s) is `s` there, and U+212A (Kelvin sign) is `k`.
    ['a long s the disk reads as s', ['manifest.json', 'manife\u017ft.json']],
    ['a Kelvin sign the disk reads as k', ['kit.txt', '\u212ait.txt']],
    // Windows drops a name's trailing dots and spaces.
    ['a trailing dot', ['manifest.json', 'manifest.json.']],
    ['a trailing space', ['manifest.json', 'manifest.json ']],
  ])('refuses %s and writes nothing', (_label, names) => {
    const dir = tempDir();
    const zip = zipOf(names);
    expect(zip.getEntries().map((entry) => entry.entryName)).toEqual(names);

    expect(() => assertDistinctZipEntries(zip)).toThrow(/name one file/);
    expect(() => safeExtractZip(zip, dir)).toThrow(/name one file/);

    expect(fs.readdirSync(dir)).toEqual([]);
  });

  // An NTFS stream (`manifest.json::$DATA` IS `manifest.json`) and an 8.3
  // short name (`MANIFE~1.JSO` can be `manifest.json`) name a file no rule
  // over the long name can see, so a bundle may not use either shape.
  it.each([
    ['an NTFS stream', 'manifest.json::$DATA'],
    ['a colon anywhere in a name', 'src/a:b.py'],
    ['an 8.3 short name', 'MANIFE~1.JSO'],
    ['an 8.3 short folder name', 'SRC~1/main.py'],
  ])('refuses %s and writes nothing', (_label, name) => {
    const dir = tempDir();
    const zip = zipOf(['manifest.json', name]);

    expect(() => assertDistinctZipEntries(zip)).toThrow(/cannot be extracted safely/);
    expect(() => safeExtractZip(zip, dir)).toThrow(/cannot be extracted safely/);

    expect(fs.readdirSync(dir)).toEqual([]);
  });

  it.each([
    ['a dotfile', '.python-version'],
    ['a name with several dots', 'src/a.b.c.py'],
    ['a dunder module', 'src/pkg/__init__.py'],
    ['a tilde that is no short name', 'docs/notes~draft.md'],
  ])('still extracts %s', (_label, name) => {
    const dir = tempDir();
    const zip = new AdmZip();
    zip.addFile('manifest.json', Buffer.from('{}'));
    zip.addFile(name, Buffer.from('kept\n'));

    expect(() => assertDistinctZipEntries(zip)).not.toThrow();
    safeExtractZip(zip, dir);

    expect(fs.readFileSync(path.join(dir, ...name.split('/')), 'utf8')).toBe('kept\n');
  });

  /**
   * The rules above read names. A disk can still alias two names no rule
   * knows (an existing hard or symbolic link inside the folder, a filesystem
   * with rules of its own), so the extraction also refuses to write an entry
   * onto a file an earlier entry of the same archive wrote.
   */
  it('refuses an entry that lands on a file an earlier entry wrote, through an existing hard link', () => {
    const dir = tempDir();
    fs.writeFileSync(path.join(dir, 'manifest.json'), 'installed\n');
    fs.linkSync(path.join(dir, 'manifest.json'), path.join(dir, 'alias.txt'));
    const zip = new AdmZip();
    zip.addFile('manifest.json', Buffer.from('checked\n'));
    zip.addFile('alias.txt', Buffer.from('written over the checked file\n'));

    expect(() => assertDistinctZipEntries(zip)).not.toThrow();
    expect(() => safeExtractZip(zip, dir)).toThrow(/name one file/);

    expect(fs.readFileSync(path.join(dir, 'manifest.json'), 'utf8')).toBe('checked\n');
  });

  it.skipIf(process.platform === 'win32')(
    'refuses an entry that lands on a file an earlier entry wrote, through an existing link inside the folder',
    () => {
      const dir = tempDir();
      fs.writeFileSync(path.join(dir, 'manifest.json'), 'installed\n');
      fs.symlinkSync(path.join(dir, 'manifest.json'), path.join(dir, 'alias.txt'));
      const zip = new AdmZip();
      zip.addFile('manifest.json', Buffer.from('checked\n'));
      zip.addFile('alias.txt', Buffer.from('written over the checked file\n'));

      expect(() => safeExtractZip(zip, dir)).toThrow(/name one file/);

      expect(fs.readFileSync(path.join(dir, 'manifest.json'), 'utf8')).toBe('checked\n');
    }
  );

  it('allows a folder entry beside the files inside it', () => {
    const dir = tempDir();
    const zip = new AdmZip();
    zip.addFile('src/', Buffer.alloc(0));
    zip.addFile('src/main.py', Buffer.from('print("ok")'));
    zip.addFile('src/util.py', Buffer.from('pass'));

    expect(() => assertDistinctZipEntries(zip)).not.toThrow();
    safeExtractZip(zip, dir);

    expect(fs.readFileSync(path.join(dir, 'src', 'util.py'), 'utf8')).toBe('pass');
  });
});

describe('extractedFileMatchesEntry', () => {
  it('is true for the file an extraction wrote from the entry', () => {
    const dir = tempDir();
    const zip = new AdmZip();
    zip.addFile('manifest.json', Buffer.from('{"name":"checked"}'));
    safeExtractZip(zip, dir);

    expect(extractedFileMatchesEntry(zip, dir, 'manifest.json')).toBe(true);
  });

  it('is false when the file on disk is not the entry, or either is missing', () => {
    const dir = tempDir();
    const zip = new AdmZip();
    zip.addFile('manifest.json', Buffer.from('{"name":"checked"}'));
    safeExtractZip(zip, dir);
    fs.writeFileSync(path.join(dir, 'manifest.json'), '{"name":"evil"}');

    expect(extractedFileMatchesEntry(zip, dir, 'manifest.json')).toBe(false);
    expect(extractedFileMatchesEntry(zip, dir, 'missing.json')).toBe(false);
    fs.rmSync(path.join(dir, 'manifest.json'));
    expect(extractedFileMatchesEntry(zip, dir, 'manifest.json')).toBe(false);
  });
});
