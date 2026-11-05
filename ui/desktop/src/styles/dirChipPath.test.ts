import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The composer footer's folder chip (spec 3.7): the folder's NAME in the
 * footer's sans, the full path in mono inside the chip's card
 * (`FolderAccessCard`, which replaced the plain path tooltip), through the
 * footer cards' one mono reading (`FooterCardValue mono`, FooterCard.tsx).
 *
 * ⚠ **Asserted at the SOURCE, and it has to be.** jsdom has no layout engine,
 * never runs Tailwind and does not implement the bidi algorithm, so a component
 * test that mounts the chip reads the same `textContent` whether the name is
 * capped or not and whether a right-to-left name reorders the line or not.
 *
 * History: the unlocked chip used to print the whole path in mono inside an
 * RTL box (`.biorouter-dir-chip-path`) that clipped the path's head. Showing a
 * name instead removed the reason for that box, and with it the two bidi bugs
 * it needed guarding against (a phantom trailing slash, `C:\` drawn `\:C`):
 * a folder name has no separators to reorder.
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

const NAME_RULE = /\.br-footline__name\s*\{([^}]*)\}/;

describe('the folder chip in the composer footer', () => {
  it('prints the folder name, not the path, in every state', () => {
    expect(code).toContain('workingDirLabel(workingDir)');
    // The chip never prints the path as text; it hands it to the card.
    expect(code).not.toMatch(/>\{workingDir\}</);
    expect(code).toMatch(/<FolderAccessCard[\s\S]*?workingDir=\{workingDir\}/);
    // The path itself reaches the screen only inside the card, through the
    // one mono reading the footer cards have.
    expect(cardCode.match(/>\{workingDir\}</g)).toHaveLength(1);
    expect(cardCode).toMatch(/<FooterCardValue mono>\{workingDir\}<\/FooterCardValue>/);
    expect(cardCode).toMatch(/<FooterCardValue mono>\{grant\.folder\}<\/FooterCardValue>/);
  });

  it('is sans: mono appears only on a path in the card', () => {
    expect(code).not.toContain('font-mono');
    expect(code).not.toContain('biorouter-dir-chip-path');
    expect(cardCode).not.toContain('font-mono');
    // One mono class among the footer cards' pieces, on the reading's mono
    // variant, which wraps a long path rather than widening the card or
    // cutting it.
    expect(surfaceCode.match(/font-mono/g)).toHaveLength(1);
    const value = surfaceCode.match(/function FooterCardValue[\s\S]*?\n\}\n/)?.[0] ?? '';
    expect(value).toContain('font-mono');
    expect(value).toContain('break-all');
    expect(PICKERS_CSS).toMatch(
      /\.br-footer-card__value\[data-mono\]\s*\{[^}]*word-break:\s*break-all/
    );
  });

  it('isolates the name so a right-to-left name cannot reorder the line', () => {
    expect(code).toMatch(/<bdi>\{workingDirLabel\(workingDir\)\}<\/bdi>/);
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
