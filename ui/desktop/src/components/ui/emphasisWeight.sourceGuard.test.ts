/**
 * No bare `<strong>` or `<b>` anywhere in the renderer's components (vision V4-R7-6; principle 3;
 * coordinator ruling 24).
 *
 * Tailwind's preflight gives `b` and `strong` `font-weight: bolder`, which computes to 700, and the
 * type system draws 400 and 600 only. Arial draws both as bold, so no screenshot shows the defect;
 * only a computed weight does (`modalEmphasisWeight.browser.test.tsx`,
 * `diagnosticsDialog.browser.test.tsx`, and the privacy, Copilot and settings ones). Those measure
 * the surfaces that had one, and each owner fixed theirs with a class. This guard holds the count at
 * zero across `components/`, so a new emphasis cannot arrive bare: each one names its weight
 * (`font-semibold`; `font-normal` for a strong that must not stand out; `font-medium`, which Arial
 * draws as 400). Ruling 24 chose this over a `b, strong { font-weight: 600 }` base rule in
 * `main.css`.
 *
 * Read from the syntax tree, not the text: a tag in a comment or a string is not an element, and
 * `<br>`, `<button>` and `<blockquote>` are not `<b>`.
 *
 * Excluded: `settings/dictation/` and `settings/tunnel/`, frozen and unmounted (nothing imports
 * them, which the last case keeps true), where four bare strongs remain that no person can see.
 */
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const SRC = resolve(here, '../..');
const COMPONENTS = resolve(here, '..');

/** Folders under `components/` that are frozen and mounted nowhere. */
const UNMOUNTED = ['settings/dictation', 'settings/tunnel'];

const NAMES_A_WEIGHT = /(?<![\w-])font-(semibold|normal|medium)(?![\w-])/;

function sources(dir: string, extension = /\.tsx$/): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return sources(path, extension);
    if (!extension.test(entry.name) || /\.(test|spec)\.tsx?$/.test(entry.name)) return [];
    return [path];
  });
}

const posix = (path: string) => path.split(sep).join('/');

/** Every `<strong>` and `<b>` element, with its line and whether its opening tag names a weight. */
export function emphasisElements(
  source: string,
  fileName = 'file.tsx'
): { line: number; tag: string; bare: boolean }[] {
  const file = ts.createSourceFile(
    fileName,
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX
  );
  const found: { line: number; tag: string; bare: boolean }[] = [];
  const visit = (node: ts.Node) => {
    if (
      (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) &&
      ts.isIdentifier(node.tagName) &&
      (node.tagName.text === 'strong' || node.tagName.text === 'b')
    ) {
      found.push({
        line: file.getLineAndCharacterOfPosition(node.getStart(file)).line + 1,
        tag: node.getText(file).replace(/\s+/g, ' '),
        bare: !NAMES_A_WEIGHT.test(node.attributes.getText(file)),
      });
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return found;
}

/** The bare ones only. */
export function bareEmphasis(source: string, fileName?: string): { line: number; tag: string }[] {
  return emphasisElements(source, fileName)
    .filter(({ bare }) => bare)
    .map(({ line, tag }) => ({ line, tag }));
}

describe('Emphasis names its weight across components/ (V4-R7-6, ruling 24)', () => {
  it('the matcher: finds a bare strong and b, and passes the ones that name 600, 400 or 500', () => {
    const sample = [
      'export const A = () => <>',
      '  <p>Click <strong>Save</strong> to keep it.</p>',
      '  <b>Bold</b>',
      '  <strong',
      '    className="text-text-default">Warning:</strong>',
      '  <strong {...rest}>Spread names nothing</strong>',
      '  <strong className="font-semibold">Fine</strong>',
      '  <b className="font-normal">Quiet</b>',
      '  <b className="font-medium tabular-nums">Medium</b>',
      "  <strong className={cn('text-text-default', 'font-semibold')}>Built</strong>",
      '  <strong className="font-semibold-ish">Not a weight</strong>',
      '  <button>Not emphasis</button><br /><blockquote>Nor this</blockquote>',
      '  {/* a <strong> in a JSX comment */}',
      '</>;',
      '// <strong> in a line comment',
      '/* <b> in a block comment */',
      "const html = '<strong>in a string</strong>';",
    ].join('\n');
    expect(bareEmphasis(sample).map((f) => f.line)).toEqual([2, 3, 4, 6, 11]);
    expect(emphasisElements(sample)).toHaveLength(9);
  });

  it('finds no bare strong or b in any component outside the unmounted folders', () => {
    const files = sources(COMPONENTS).filter(
      (file) =>
        !UNMOUNTED.some((folder) => posix(relative(COMPONENTS, file)).startsWith(`${folder}/`))
    );
    expect(files.length).toBeGreaterThan(300);
    const all = files.flatMap((file) =>
      emphasisElements(readFileSync(file, 'utf8'), file).map((found) => ({
        ...found,
        file: posix(relative(COMPONENTS, file)),
      }))
    );
    // Not vacuous: the scan sees the emphasis the owners fixed (privacy, settings, ui, Copilot).
    expect(all.filter(({ bare }) => !bare).length).toBeGreaterThan(10);
    expect(
      all.filter(({ bare }) => bare).map(({ file, line, tag }) => `${file}:${line} ${tag}`),
      'Name the weight: className="font-semibold" (or font-normal for a strong that must not ' +
        'stand out). A bare <strong> or <b> computes to 700, which the type system does not draw.'
    ).toEqual([]);
  });

  it('the excluded folders are still mounted nowhere, which is what lets them stay', () => {
    const importers = sources(SRC, /\.tsx?$/).filter((file) => {
      const path = posix(relative(COMPONENTS, file));
      if (UNMOUNTED.some((folder) => path.startsWith(`${folder}/`))) return false;
      const text = readFileSync(file, 'utf8');
      return UNMOUNTED.some((folder) => {
        const name = folder.split('/').pop();
        return new RegExp(`(from\\s+|import\\s*\\(\\s*)['"][^'"]*\\b${name}/[^'"]+['"]`).test(text);
      });
    });
    expect(importers.map((file) => posix(relative(SRC, file)))).toEqual([]);
  });
});
