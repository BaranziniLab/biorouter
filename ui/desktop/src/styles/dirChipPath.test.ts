import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The composer footer's folder chip (spec 3.7): the folder's NAME in the
 * footer's sans, the full path in a mono tooltip.
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
const PICKERS_CSS = readFileSync(join(__dirname, '../components/bottom_menu/pickers.css'), 'utf8');

/** Code only, so a comment that names a class cannot satisfy or trip a rule. */
const code = DIR_SWITCHER.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');

const NAME_RULE = /\.br-footline__name\s*\{([^}]*)\}/;

describe('the folder chip in the composer footer', () => {
  it('prints the folder name, not the path, in both states', () => {
    expect(code).toContain('workingDirLabel(workingDir)');
    // The path itself reaches the screen only inside the tooltip.
    expect(code.match(/>\{workingDir\}</g)).toHaveLength(1);
    expect(code).toMatch(/<span className="font-mono">\{workingDir\}<\/span>/);
  });

  it('is sans: mono appears only on the tooltip path', () => {
    expect(code.match(/font-mono/g)).toHaveLength(1);
    expect(code).not.toContain('biorouter-dir-chip-path');
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

  it('uses the footer item recipe in both states', () => {
    expect(code.match(/br-footline__item/g)?.length).toBeGreaterThanOrEqual(2);
    expect(code).toContain('data-testid="dir-switcher-locked"');
  });
});
