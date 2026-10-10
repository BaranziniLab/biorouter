import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { DOCUMENT_FIDELITY_NOTES, describeUnsupportedFormat } from './formatSupport';

/**
 * The panel answered every unsupported file with the same sentence. These tests
 * pin the replacement: a *specific* refusal that names the format and says what
 * to do instead.
 */
describe('describeUnsupportedFormat', () => {
  it('returns nothing for a format the panel actually renders', () => {
    for (const extension of ['pdf', 'docx', 'xlsx', 'pptx', 'png', 'svg', 'md', 'csv']) {
      expect(describeUnsupportedFormat(extension), extension).toBeNull();
    }
  });

  it('returns nothing for an extension it has no specific complaint about', () => {
    // The generic message is honest here, so inventing a reason would be worse.
    expect(describeUnsupportedFormat('bin')).toBeNull();
    expect(describeUnsupportedFormat(undefined)).toBeNull();
    expect(describeUnsupportedFormat('')).toBeNull();
  });

  // The distinction that actually confuses users: .doc is not "an old .docx",
  // it is a different container entirely, and no permissive browser renderer
  // for it exists.
  it.each(['doc', 'xls', 'ppt'])('explains that legacy .%s is a different format', (extension) => {
    const note = describeUnsupportedFormat(extension)!;
    expect(note.label).toMatch(/^Legacy/);
    expect(note.reason).toContain('different container');
    expect(note.suggestion).toBeTruthy();
  });

  it.each(['odt', 'ods', 'odp', 'rtf', 'pages', 'numbers', 'key'])(
    'declines .%s with a reason and a way forward',
    (extension) => {
      const note = describeUnsupportedFormat(extension)!;
      expect(note.label).toBeTruthy();
      expect(note.reason).toBeTruthy();
      expect(note.suggestion).toBeTruthy();
    }
  );

  it.each(['heic', 'heif'])('names .%s as an image nothing here can decode', (extension) => {
    const note = describeUnsupportedFormat(extension)!;
    expect(note.label).toMatch(/image$/);
    expect(note.suggestion).toContain('PNG');
  });

  // TIFF is decoded in the renderer now, so a TIFF that fails is a broken file
  // rather than an unsupported format — and claiming otherwise would send the
  // user off to convert a file that would have worked.
  it.each(['tif', 'tiff'])('no longer refuses .%s, because it decodes', (extension) => {
    expect(describeUnsupportedFormat(extension)).toBeNull();
  });

  it('is case-insensitive, because file names are not', () => {
    expect(describeUnsupportedFormat('DOC')?.label).toBe('Legacy Word document');
    expect(describeUnsupportedFormat('HEIC')?.label).toBe('HEIC image');
  });

  it('never apologises or hedges', () => {
    for (const extension of ['doc', 'odt', 'heic', 'key']) {
      const note = describeUnsupportedFormat(extension)!;
      expect(`${note.reason} ${note.suggestion}`).not.toMatch(/sorry|unfortunately|afraid/i);
    }
  });
});

describe('the fidelity ceiling is stated, not discovered', () => {
  it('covers every format the panel renders', () => {
    expect(Object.keys(DOCUMENT_FIDELITY_NOTES).sort()).toEqual(['docx', 'pdf', 'pptx', 'xlsx']);
  });

  // The specific limits users hit first, and mistake for a corrupt file.
  it('names the blind spots that get reported as bugs', () => {
    expect(DOCUMENT_FIDELITY_NOTES.pptx).toContain('animations');
    expect(DOCUMENT_FIDELITY_NOTES.docx).toContain('table of contents');
    expect(DOCUMENT_FIDELITY_NOTES.xlsx).toContain('formulas');
  });

  it('is rendered by the preview rather than only documented', () => {
    const source = readFileSync(
      join(__dirname, '..', 'components', 'artifacts', 'DocumentPreview.tsx'),
      'utf8'
    );
    expect(source).toContain('DOCUMENT_FIDELITY_NOTES');
    expect(source).toContain('document-fidelity-note');
  });

  it('is wired into the panel’s refusal card', () => {
    const source = readFileSync(
      join(__dirname, '..', 'components', 'artifacts', 'ArtifactViewer.tsx'),
      'utf8'
    );
    expect(source).toContain('describeUnsupportedFormat');
    // The old one-size-fits-all sentence must only survive as the fallback:
    // the false arm of the ternary whose true arm is the format's own reason.
    // Whitespace-free, so a Prettier line break does not decide the verdict.
    expect(source).toMatch(
      /unsupported\s*\?\s*unsupported\.reason\s*:\s*PREVIEW_COPY\.binaryFallbackReason\b/
    );
  });
});
