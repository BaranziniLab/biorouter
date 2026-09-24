import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The old Crew layout stays deleted.
 *
 * `/crew` renders only `CrewApp`, so the old layout (`crew/legacy/LegacyCrewLayout.tsx`), its
 * stylesheet (`crew/crew.css`) and its file controls (`crew/CrewFiles.tsx`) were deleted rather
 * than left compiled. The stylesheet is the reason this is guarded. It was global: once any module
 * on the page imported it, its unlayered rules applied everywhere, and it styled `.crew-main`,
 * `.crew-channel`, `.crew-timeline`, `.crew-message-meta`, `.crew-message-body`, `.crew-composer`
 * and `.crew-attachment`, names the redesigned layout also uses. A render test cannot hold that,
 * because jsdom would load the sheet and apply it without complaint.
 *
 * So the guard is at the source. None of those paths exists, and no source file names one in an
 * import, a re-export, a dynamic `import()`, a `vi.mock` / `vi.importActual`, or a CSS `@import`.
 * The same holds for the vitest and vite configs beside `src/`.
 *
 * The matcher is exercised on fixtures below, so the guard cannot pass by failing to look.
 */

const THIS_FILE = resolve(__dirname, 'legacyStylesheet.test.ts');
const CREW_DIR = resolve(__dirname, '..');
const SRC_DIR = resolve(CREW_DIR, '../..');
const DESKTOP_DIR = dirname(SRC_DIR);

const DELETED_DIR = join(CREW_DIR, 'legacy');
const DELETED_FILES = [
  join(CREW_DIR, 'crew.css'),
  join(CREW_DIR, 'CrewFiles.tsx'),
  join(CREW_DIR, 'CrewFiles.regression.test.tsx'),
];
/** Module paths as a specifier names them: without an extension, except for a stylesheet. */
const DELETED_MODULES = [join(CREW_DIR, 'crew.css'), join(CREW_DIR, 'CrewFiles')];

const SOURCE_EXTENSION = /\.(?:[cm]?[jt]sx?|css)$/;
/** Built bundles and staged binaries: never source, and far too large to read. */
const SKIPPED_DIRS = new Set([join(SRC_DIR, 'bin'), join(SRC_DIR, 'web')]);

/**
 * A string literal in a module-specifier position: `from '…'`, a side-effect `import '…'`, a
 * dynamic `import('…')`, `vi.mock('…')` (and `doMock` / `unmock`),
 * `vi.importActual<…>('…')`, `require('…')` and a CSS `@import '…'` / `@import url('…')`.
 */
const SPECIFIER_PATTERN =
  /(?:\bfrom\s*|@import\s+(?:url\(\s*)?|\bimport\s*\(?\s*|\bvi\.(?:do|un)?[mM]ock\(\s*|\bimportActual\s*(?:<[^>]*>)?\s*\(\s*|\brequire\s*\(\s*)['"]([^'"]+)['"]/g;

function stripComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, ' ').replace(/(^|[^:])\/\/[^\n]*/g, '$1');
}

function walk(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      return entry.name === 'node_modules' || SKIPPED_DIRS.has(path) ? [] : walk(path);
    }
    return SOURCE_EXTENSION.test(entry.name) ? [path] : [];
  });
}

/** The absolute path a relative or `@/` specifier names, or `null` for a package. */
function specifierPath(from: string, specifier: string): string | null {
  if (specifier.startsWith('@/')) return join(SRC_DIR, specifier.slice(2));
  if (specifier.startsWith('.')) return resolve(dirname(from), specifier);
  return null;
}

function namesDeleted(path: string): boolean {
  if (path === DELETED_DIR || path.startsWith(DELETED_DIR + sep)) return true;
  const withoutScript = path.replace(/\.(?:[cm]?[jt]sx?)$/, '');
  return DELETED_MODULES.includes(withoutScript);
}

/** Every specifier in `source` (a file at `from`) that names a deleted Crew path. */
function deletedReferences(from: string, source: string): string[] {
  return [...stripComments(source).matchAll(SPECIFIER_PATTERN)]
    .map((match) => match[1])
    .filter((specifier) => {
      const path = specifierPath(from, specifier);
      return path !== null && namesDeleted(path);
    });
}

const rel = (path: string) => relative(DESKTOP_DIR, path).split(sep).join('/');

// This file is left out: its fixtures name the deleted paths on purpose.
const SOURCE_FILES = walk(SRC_DIR).filter((path) => path !== THIS_FILE);
const CONFIG_FILES = readdirSync(DESKTOP_DIR)
  .filter((name) => /^(?:vite|vitest)[\w.-]*\.(?:[cm]?[jt]s)$/.test(name))
  .map((name) => join(DESKTOP_DIR, name));

describe('the old Crew layout stays deleted', () => {
  it('has no crew/legacy/ directory, no crew.css and no CrewFiles', () => {
    expect(existsSync(DELETED_DIR)).toBe(false);
    expect(DELETED_FILES.filter((path) => existsSync(path)).map(rel)).toEqual([]);
  });

  it('is named by no import, mock or @import anywhere in src/ or the build configs', () => {
    // The walk really covered the Crew tree, its stylesheets and the rest of the app.
    expect(SOURCE_FILES.map(rel)).toEqual(
      expect.arrayContaining([
        'src/App.tsx',
        'src/components/crew/CrewApp.tsx',
        'src/components/crew/crew-app.css',
        'src/components/crew/CrewView.regression.test.tsx',
        'src/components/crew/layout/CrewLayout.tsx',
      ])
    );
    expect(CONFIG_FILES.map(rel)).toEqual(
      expect.arrayContaining(['vitest.config.ts', 'vite.renderer.config.mts'])
    );

    const found = [...SOURCE_FILES, ...CONFIG_FILES].flatMap((path) =>
      deletedReferences(path, readFileSync(path, 'utf8')).map(
        (specifier) => `${rel(path)}: ${specifier}`
      )
    );
    expect(found).toEqual([]);
  });

  it('routes /crew to CrewApp', () => {
    const app = stripComments(readFileSync(join(SRC_DIR, 'App.tsx'), 'utf8'));
    expect(app).toMatch(/<Route\s+path="crew"\s+element=\{<CrewApp\s*\/>\}\s*\/>/);
  });
});

describe('the guard itself', () => {
  const view = join(CREW_DIR, 'CrewView.tsx');
  const areaCss = join(CREW_DIR, 'files', 'files.css');
  const outside = join(SRC_DIR, 'App.tsx');

  it('finds every way a file can name a deleted path', () => {
    expect(
      deletedReferences(
        view,
        [
          "import LegacyCrewLayout from './legacy/LegacyCrewLayout';",
          "export { default } from './legacy';",
          "import './crew.css';",
          "const lazy = () => import('./legacy/LegacyCrewLayout.tsx');",
          "vi.mock('./CrewFiles', () => ({ CrewUpload: () => null }));",
          "vi.doMock('./CrewFiles.tsx');",
          "await vi.importActual<typeof import('./CrewFiles')>('./CrewFiles');",
          "import type { CrewUploadProps } from './CrewFiles';",
          "require('./crew.css');",
        ].join('\n')
      )
    ).toEqual([
      './legacy/LegacyCrewLayout',
      './legacy',
      './crew.css',
      './legacy/LegacyCrewLayout.tsx',
      './CrewFiles',
      './CrewFiles.tsx',
      './CrewFiles',
      './CrewFiles',
      './crew.css',
    ]);
    expect(
      deletedReferences(areaCss, '@import \'../crew.css\';\n@import url("../crew.css");')
    ).toEqual(['../crew.css', '../crew.css']);
    expect(
      deletedReferences(
        outside,
        [
          "import Legacy from './components/crew/legacy/LegacyCrewLayout';",
          "import '@/components/crew/crew.css';",
        ].join('\n')
      )
    ).toEqual(['./components/crew/legacy/LegacyCrewLayout', '@/components/crew/crew.css']);
  });

  it('ignores live modules, packages and prose', () => {
    expect(
      deletedReferences(
        view,
        [
          "import CrewApp from './CrewApp';",
          "import './crew-app.css';",
          "import { crewTransfers } from './crewTransfers';",
          "import { useCrewUpload } from './files/useCrewUpload';",
          "import worker from 'pdfjs-dist/legacy/build/pdf.worker.min.mjs';",
          "// The old layout imported './crew.css' and './legacy/LegacyCrewLayout'.",
          "/* vi.mock('./CrewFiles') was removed with it. */",
          "const note = 'crew.css';",
        ].join('\n')
      )
    ).toEqual([]);
    // A path that only starts like the deleted directory is not in it.
    expect(deletedReferences(view, "import x from './legacyNotes';")).toEqual([]);
  });
});
