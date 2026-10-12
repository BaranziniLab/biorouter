/**
 * Every dialog is a `ModalShell` (spec 2.6, V3-R7-1): no production file outside the shell renders
 * `DialogContent` itself.
 *
 * The primitive no longer carries a `p-6 gap-4 sm:max-w-lg` default (`dialog.test.tsx`), so a raw
 * content now opens with no insets, centred, at the full small-screen width: visibly wrong, but only
 * in a person's hands. This guard says so at the source, naming the file and line, the day one
 * arrives. The redesign found 34 raw sites (spec 2.6) and moved them owner by owner; the last, the
 * non-private model disclosure, moved under ruling 23.
 *
 * The one exception is `settings/tunnel/TunnelSection.tsx`: frozen and unmounted (nothing imports
 * it), so no person can open its dialogs. The last case keeps that true; if the folder is ever
 * mounted again, its dialogs move onto the shell and the exception goes.
 */
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const SRC = resolve(here, '../..');
const COMPONENTS = resolve(here, '..');

/** The shell itself, and the frozen, unmounted tunnel section. Paths from `components/`. */
const ALLOWED = new Set(['ModalShell.tsx', 'settings/tunnel/TunnelSection.tsx']);

function productionSources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return productionSources(path);
    if (!/\.tsx?$/.test(entry.name) || /\.(test|spec)\.tsx?$/.test(entry.name)) return [];
    return [path];
  });
}

/**
 * Every JSX use of the dialog primitive's `DialogContent`, under any local name it was imported
 * as, with its line. A `DialogContent` from anywhere else (a sheet, a library) is not this one.
 */
export function rawDialogContents(source: string, fileName = 'file.tsx'): number[] {
  const file = ts.createSourceFile(
    fileName,
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX
  );
  const local = new Set<string>();
  for (const statement of file.statements) {
    if (!ts.isImportDeclaration(statement)) continue;
    const from = (statement.moduleSpecifier as ts.StringLiteral).text;
    if (!/(^|\/)(ui\/)?dialog$/.test(from)) continue;
    const named = statement.importClause?.namedBindings;
    if (!named || !ts.isNamedImports(named)) continue;
    for (const specifier of named.elements) {
      if ((specifier.propertyName ?? specifier.name).text === 'DialogContent') {
        local.add(specifier.name.text);
      }
    }
  }
  const lines: number[] = [];
  const visit = (node: ts.Node) => {
    if (
      (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) &&
      ts.isIdentifier(node.tagName) &&
      local.has(node.tagName.text)
    ) {
      lines.push(file.getLineAndCharacterOfPosition(node.getStart(file)).line + 1);
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return lines;
}

describe('No raw DialogContent outside ModalShell (spec 2.6)', () => {
  it('the matcher: finds the primitive under its own name and an alias, and nothing else', () => {
    const sample = [
      "import { DialogContent, DialogTitle } from '../ui/dialog';",
      "import { DialogContent as Surface } from './dialog';",
      "import { SheetContent } from '../ui/sheet';",
      'export const A = () => <DialogContent><DialogTitle>a</DialogTitle></DialogContent>;',
      'export const B = () => <Surface />;',
      'export const C = () => <SheetContent />;',
      '// <DialogContent> in a comment is not a use',
      "export const D = () => '<DialogContent>';",
    ].join('\n');
    expect(rawDialogContents(sample)).toEqual([4, 5]);
    // The same tag imported from somewhere that is not the primitive is not counted.
    expect(
      rawDialogContents(
        "import { DialogContent } from 'some-library';\nconst E = () => <DialogContent />;"
      )
    ).toEqual([]);
  });

  it('finds none in production code except the shell and the unmounted tunnel section', () => {
    const files = productionSources(COMPONENTS).filter((file) => file.endsWith('.tsx'));
    expect(files.length).toBeGreaterThan(300);
    const found = files.flatMap((file) => {
      const name = relative(COMPONENTS, file);
      return rawDialogContents(readFileSync(file, 'utf8'), name).map((line) => ({ name, line }));
    });
    const offenders = found.filter(({ name }) => !ALLOWED.has(name));
    expect(
      offenders.map(({ name, line }) => `${name}:${line}`),
      'Render the dialog through ModalShell (components/ModalShell.tsx): it owns the size, the ' +
        '16px insets, the header and footer, and the dismissal rule (purpose).'
    ).toEqual([]);
    // Not vacuous: the shell is found, as is each of the tunnel section's two dialogs.
    expect(found.filter(({ name }) => name === 'ModalShell.tsx')).toHaveLength(1);
    expect(found.filter(({ name }) => name === 'settings/tunnel/TunnelSection.tsx')).toHaveLength(
      2
    );
  });

  it('the tunnel section is still unmounted, which is what lets it stay raw', () => {
    const importers = productionSources(SRC).filter((file) => {
      if (file.includes(`${join('settings', 'tunnel')}/`)) return false;
      return /(from\s+|import\s*\(\s*)['"][^'"]*tunnel\/TunnelSection['"]/.test(
        readFileSync(file, 'utf8')
      );
    });
    expect(importers.map((file) => relative(SRC, file))).toEqual([]);
  });
});
