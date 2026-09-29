import { render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { extensionOf, middleTruncate, MIDDLE_ELLIPSIS } from './middleName';
import { MiddleTruncatedName } from './MiddleTruncatedName';

/** One unit of width per character: what a monospaced font draws. */
const perCharacter = (text: string) => Array.from(text).length;

/**
 * FILES2-N1: a card cut a long name at its end, so `q3-report.pdf…` hid the `.exe` it ended with.
 * A name is cut in its middle now, and its extension always stays.
 */
describe('middleTruncate', () => {
  it('keeps a name that fits whole', () => {
    expect(middleTruncate('counts.csv', 10, perCharacter)).toBe('counts.csv');
    expect(middleTruncate('counts.csv', 0, perCharacter)).toBe('counts.csv');
  });

  it('cuts a long name in its middle, keeping its extension and a little before it', () => {
    const name = `q3-report.pdf${'_'.repeat(40)}.exe`;
    const shown = middleTruncate(name, 24, perCharacter);
    expect(perCharacter(shown)).toBeLessThanOrEqual(24);
    expect(shown.startsWith('q3-report.pdf')).toBe(true);
    expect(shown.endsWith('____.exe')).toBe(true);
    expect(shown).toContain(MIDDLE_ELLIPSIS);
  });

  it('keeps the ellipsis and the ending even when nothing of the start fits', () => {
    expect(middleTruncate('abcdefghij.csv', 3, perCharacter)).toBe(`${MIDDLE_ELLIPSIS}ghij.csv`);
  });

  it('never splits a character outside the Basic Multilingual Plane', () => {
    const name = `${'\u{1F9EC}'.repeat(20)}.fa`;
    const shown = middleTruncate(name, 10, perCharacter);
    expect(Array.from(shown).every((character) => character.length > 0)).toBe(true);
    expect(
      /[\uD800-\uDFFF](?![\uDC00-\uDFFF])/.test(
        shown.replace(/[\uD800-\uDBFF][\uDC00-\uDFFF]/g, '')
      )
    ).toBe(false);
    expect(shown.endsWith('.fa')).toBe(true);
  });
});

describe('extensionOf', () => {
  it('is the ending from the last dot, when that is short and not the first character', () => {
    expect(extensionOf('report.pdf')).toBe('.pdf');
    expect(extensionOf('archive.tar.gz')).toBe('.gz');
    expect(extensionOf('.Rprofile')).toBe('');
    expect(extensionOf('README')).toBe('');
    expect(extensionOf('notes.averyveryverylongending')).toBe('');
  });
});

describe('MiddleTruncatedName', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('draws a name too wide for its label cut in the middle, and a name that fits whole', () => {
    // jsdom lays nothing out: every label is 140px wide, and every character 7px.
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(140);
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({
      font: '',
      measureText: (text: string) => ({ width: Array.from(text).length * 7 }),
    } as unknown as CanvasRenderingContext2D);
    const long = `q3-report.pdf${'_'.repeat(40)}.exe`;
    const { container, rerender } = render(
      <span className="crew-attachment-label">
        <MiddleTruncatedName name={long} className="crew-attachment-name" />
      </span>
    );
    const shown = container.querySelector('[data-crew-file-name]')?.textContent ?? '';
    expect(Array.from(shown).length).toBeLessThanOrEqual(20);
    expect(shown.startsWith('q3-report')).toBe(true);
    expect(shown.endsWith('.exe')).toBe(true);
    rerender(
      <span className="crew-attachment-label">
        <MiddleTruncatedName name="counts.csv" className="crew-attachment-name" />
      </span>
    );
    expect(container.querySelector('[data-crew-file-name]')).toHaveTextContent('counts.csv');
  });
});
