import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The composer footer's folder chip (spec 3.7) and the folder in its card: the folder's NAME on
 * the chip, in the footer's sans, and the whole location in the chip's card (`FolderAccessCard`,
 * which replaced the plain path tooltip), through the footer cards' one folder reading
 * (`FooterCardPath`, FooterCard.tsx). Both are written by the one rule in utils/folderPath.ts:
 * `~` for the home folder, the sidebar's word for the name.
 *
 * ⚠ **Asserted at the SOURCE, and it has to be.** jsdom has no layout engine, never runs Tailwind
 * and does not implement the bidi algorithm, so a component test that mounts the chip reads the
 * same `textContent` whether the name is capped or not and whether a right-to-left name reorders
 * the line or not. Chromium measures the wrap and the face (footerCard.browser.test.tsx).
 *
 * History: the unlocked chip used to print the whole path in mono inside an RTL box
 * (`.biorouter-dir-chip-path`) that clipped the path's head; then the card set the path in mono
 * with `break-all`, a 13px face that read larger than the card around it and split names between
 * any two letters. The path is now a value like the card's others, isolated name by name.
 */
const DIR_SWITCHER = readFileSync(
  join(__dirname, '../components/bottom_menu/DirSwitcher.tsx'),
  'utf8'
);
const FOLDER_CARD = readFileSync(
  join(__dirname, '../components/bottom_menu/FolderAccessCard.tsx'),
  'utf8'
);
const FOOTER_CARD = readFileSync(
  join(__dirname, '../components/bottom_menu/FooterCard.tsx'),
  'utf8'
);
const PICKERS_CSS = readFileSync(join(__dirname, '../components/bottom_menu/pickers.css'), 'utf8');

/** Code only, so a comment that names a class cannot satisfy or trip a rule. */
const codeOnly = (source: string) =>
  source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
const code = codeOnly(DIR_SWITCHER);
const cardCode = codeOnly(FOLDER_CARD);
const surfaceCode = codeOnly(FOOTER_CARD);
const PICKERS_CSS_CODE = PICKERS_CSS.replace(/\/\*[\s\S]*?\*\//g, '');

const NAME_RULE = /\.br-footline__name\s*\{([^}]*)\}/;

describe('the folder chip in the composer footer', () => {
  it('prints the folder name, not the path, in every state', () => {
    expect(code).toContain('folderName(workingDir, homeFolder())');
    // The chip never prints the path as text; it hands it to the card.
    expect(code).not.toMatch(/>\{workingDir\}</);
    expect(code).toMatch(/<FolderAccessCard[\s\S]*?workingDir=\{workingDir\}/);
    // The path itself reaches the screen only inside the card, through the one folder reading the
    // footer cards have, for the working folder and for every grant.
    expect(cardCode).not.toMatch(/>\{(workingDir|grant\.folder)\}</);
    expect(cardCode).toMatch(/<FooterCardPath path=\{workingDir\} \/>/);
    expect(cardCode).toMatch(/<FooterCardPath path=\{grant\.folder\} \/>/);
    expect(cardCode.match(/<FooterCardPath\b/g)).toHaveLength(2);
  });

  it('writes the path as the app writes a folder: `~` for home, in the card’s value', () => {
    const piece = surfaceCode.match(/export function FooterCardPath[\s\S]*?\n\}\n/)?.[0] ?? '';
    expect(piece, 'FooterCard.tsx has a FooterCardPath').not.toBe('');
    expect(piece).toContain('tildePath(path, homeFolder())');
    // It IS a value: the same piece, plus the path's own class.
    expect(piece).toMatch(/<FooterCardValue\b[^>]*className="br-footer-card__path"/);
    // A left-to-right line, the absolute path kept for the record, each name in its own isolate
    // and the separators outside them.
    expect(piece).toMatch(/dir="ltr"/);
    expect(piece).toMatch(/data-path=\{path\}/);
    // Drawn as the transcript's folder-access approval draws its folder (ui/path-segments.tsx):
    // each name isolated, and each name with the separator after it one piece a line never breaks
    // inside. The markup is FolderAccessCard.test.tsx's to check, the line breaks
    // footerCard.browser.test.tsx's.
    expect(piece).toMatch(/<PathSegments path=\{tildePath\(path, homeFolder\(\)\)\} \/>/);
  });

  it('is sans everywhere: no mono and no break-all in the chip, the card or its pieces', () => {
    for (const [name, source] of [
      ['DirSwitcher.tsx', code],
      ['FolderAccessCard.tsx', cardCode],
      ['FooterCard.tsx', surfaceCode],
    ] as const) {
      expect(source, name).not.toContain('font-mono');
      expect(source, name).not.toContain('break-all');
      expect(source, name).not.toContain('biorouter-dir-chip-path');
    }
    // FooterCardValue lost its mono variant.
    const value = surfaceCode.match(/export function FooterCardValue[\s\S]*?\n\}\n/)?.[0] ?? '';
    expect(value, 'FooterCard.tsx has a FooterCardValue').not.toBe('');
    expect(value).not.toMatch(/mono/);
    expect(PICKERS_CSS_CODE).not.toMatch(/data-mono/);
    expect(PICKERS_CSS_CODE).not.toMatch(/word-break:\s*break-all/);
  });

  it('wraps a folder in authored CSS, inside a name only as the last resort', () => {
    const rule = PICKERS_CSS_CODE.match(/\.br-footer-card__path\s*\{([^}]*)\}/)?.[1];
    expect(rule).toBeTruthy();
    expect(rule).toMatch(/overflow-wrap:\s*anywhere/);
    // Never cut and never set apart: no nowrap, no ellipsis, no face or ink of its own.
    expect(rule).not.toMatch(/white-space|text-overflow|font-family|color/);
    // A name and the separator after it are one atomic box no wider than the line, so a line
    // breaks between two of them (after a separator), not after a hyphen or at a space in a name.
    const piece = PICKERS_CSS_CODE.match(
      /\.br-footer-card__path \.br-path-piece\s*\{([^}]*)\}/
    )?.[1];
    expect(piece).toBeTruthy();
    expect(piece).toMatch(/display:\s*inline-block/);
    expect(piece).toMatch(/max-width:\s*100%/);
    expect(piece).not.toMatch(/white-space|text-overflow/);
  });

  it('centres a row’s action on its value’s first line, from the tokens', () => {
    const rule = PICKERS_CSS_CODE.match(/\.br-footer-card__row-action\s*\{([^}]*)\}/)?.[1];
    expect(rule).toBeTruthy();
    expect(rule).toMatch(
      /margin-top:\s*calc\(\(var\(--text-secondary--line-height\) - var\(--control-compact\)\) \/ 2\)/
    );
  });

  it('isolates the name so a right-to-left name cannot reorder the line', () => {
    expect(code).toMatch(/<bdi>\{shownName\}<\/bdi>/);
  });

  it('caps the name in authored CSS and ends it with an ellipsis', () => {
    const rule = PICKERS_CSS.match(NAME_RULE)?.[1];
    expect(rule).toBeTruthy();
    expect(rule).toMatch(/max-width:\s*\d+px/);
    expect(rule).toMatch(/text-overflow:\s*ellipsis/);
    // `min-width: 0` is what lets flex shrink the chip before the readouts.
    expect(rule).toMatch(/min-width:\s*0/);
  });

  it('is one trigger on the footer item recipe in every state', () => {
    // One `FooterCardTrigger`, the same for every footer item, carries the
    // recipe, so the chip's states (and the line's three items) cannot drift.
    expect(code.match(/<FooterCardTrigger\b/g)).toHaveLength(1);
    expect(code).not.toMatch(/<button\b/);
    expect(surfaceCode).toMatch(/className=\{cn\('br-footline__item', className\)\}/);
    // The state names the chip for tests only; its behaviour is the same.
    expect(code).toMatch(
      /workingDir === null\s*\?\s*'dir-switcher-unknown'\s*:\s*locked\s*\?\s*'dir-switcher-locked'\s*:\s*'dir-switcher'/
    );
  });
});
