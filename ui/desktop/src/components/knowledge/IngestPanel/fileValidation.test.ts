import { describe, expect, it } from 'vitest';

import { MAX_INGEST_FILE_BYTES, formatBytes, validateDroppedFiles } from './fileValidation';

/** A dropped file of the given name, claiming the given size without allocating it. */
function dropped(name: string, size = 10) {
  const file = new File(['x'], name);
  Object.defineProperty(file, 'size', { value: size });
  return { file };
}

/**
 * Owner, 2026-11-13: no blocks of text. A staging warning is a title and ONE line under it that
 * names the file and adds what the title does not say, never a sentence restating the title.
 */
describe('validateDroppedFiles: one line under each warning title', () => {
  it('names the file and keeps both sizes for a file that is too large', () => {
    const { warnings, accepted } = validateDroppedFiles([
      dropped('scan.pdf', MAX_INGEST_FILE_BYTES + 1),
    ]);
    expect(accepted).toHaveLength(0);
    expect(warnings).toHaveLength(1);
    expect(warnings[0].level).toBe('error');
    expect(warnings[0].message).toBe(
      `scan.pdf is ${formatBytes(MAX_INGEST_FILE_BYTES + 1)}; the limit for this kind of file is ${formatBytes(MAX_INGEST_FILE_BYTES)}.`
    );
  });

  it('sends a knowledge base archive to the importer by the button that does it', () => {
    const { warnings, accepted } = validateDroppedFiles([dropped('notes.brkb')]);
    expect(accepted).toHaveLength(0);
    expect(warnings[0].title).toBe('Use the knowledge base importer for archives');
    expect(warnings[0].message).toBe(
      'notes.brkb is a knowledge base archive: use "Import from .brkb" in Manage bases instead.'
    );
  });

  it('leaves a binary out and says so in one clause', () => {
    const { warnings, accepted } = validateDroppedFiles([dropped('setup.exe')]);
    expect(accepted).toHaveLength(0);
    expect(warnings[0].message).toBe(
      'setup.exe looks like an executable or compiled binary, so it was left out.'
    );
  });

  it('stages an unknown format as text and says so without restating the title', () => {
    const { warnings, accepted } = validateDroppedFiles([dropped('trace.xyz')]);
    expect(accepted).toHaveLength(1);
    expect(warnings[0].title).toBe('Staging as plain text');
    expect(warnings[0].message).toBe(
      'trace.xyz is not a common ingest format, so Biorouter will read it as text if it can.'
    );
  });

  it('keeps every message to one sentence with no dash', () => {
    const { warnings } = validateDroppedFiles([
      dropped('scan.pdf', MAX_INGEST_FILE_BYTES + 1),
      dropped('notes.brkb'),
      dropped('setup.exe'),
      dropped('trace.xyz'),
    ]);
    for (const warning of warnings) {
      expect(warning.message.match(/[.!?](\s|$)/g) ?? []).toHaveLength(1);
      expect(warning.message).not.toMatch(/[–—]/);
    }
  });
});
