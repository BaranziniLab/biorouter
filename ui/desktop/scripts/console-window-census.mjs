/**
 * The console-window census for the Electron main process.
 *
 * # Why this exists
 *
 * The main process is a GUI-subsystem process and owns no console. Every
 * CONSOLE-subsystem child it starts without `CREATE_NO_WINDOW` makes Windows
 * allocate a new, VISIBLE console for that child — so a short-lived probe
 * flashes a black box on the user's screen. Node's knob is the spawn option
 * `windowsHide`, and its default is `false`, which means the damaging behaviour
 * is what you get by writing nothing (#368).
 *
 * The Rust side learned this first and is guarded by
 * `crates/biorouter-mcp/tests/no_console_window_census.rs`. This is the same
 * tripwire for the half of the app that census cannot see.
 *
 * # The two rules, and which one is load-bearing — measured, not argued
 *
 * Both were rewritten after the answers came back from real Windows (Electron
 * 39.8.10, 2026-09-22), because reading the source alone got the emphasis
 * exactly backwards twice.
 *
 * **Rule 1: every site states `windowsHide` explicitly.** This is the
 * load-bearing one. There are two independent levers and they cover different
 * sites: `CREATE_NO_WINDOW` gives a child no console at all but libuv applies
 * it only when no stdio entry is an inherited fd, while `SW_HIDE` — which
 * `windowsHide: true` also requests, because libuv sets STARTF_USESHOWWINDOW
 * unconditionally — hides a console that did get created. So an inherit-stdio
 * spawn falls through the first lever and is caught by the second alone.
 * Measured: inherit + `windowsHide: true` reports `hidden` (a console exists,
 * no window), where a non-inheriting spawn reports `none` (no console at all).
 * The one shape that puts a black box on screen is a site that inherits an fd
 * AND omits the option.
 *
 * ⚠ It is easy to conclude the option does nothing, and that conclusion is
 * wrong for a reason worth writing down. Electron sets
 * `EnvironmentFlags::kHideConsoleWindows` on every Node environment it creates
 * (shell/common/node_bindings.cc, unconditionally, since Electron 16), so a
 * piped spawn from the main process is already hidden with no `windowsHide` at
 * all — confirmed by measurement, not inferred. That is why nothing in
 * `ui/desktop/src` was drawing a black box when this was investigated, and it
 * is precisely what makes the option look decorative. It is not: it is the only
 * thing standing between an inherit-stdio site and a visible console, and it is
 * the only thing that does not depend on an embedder detail Electron could drop.
 *
 * **Rule 2: no site inherits a standard handle.** Defence in depth, and a
 * strictly stronger outcome where it holds (`none` beats `hidden`: there is no
 * console to show, so nothing can later reveal it). It is NOT justified by "an
 * inheriting site draws a window" — measurement says it does not, so long as
 * rule 1 holds. It is justified by not wanting the two rules to be load-bearing
 * one at a time.
 *
 * # What it asserts
 *
 * Every call to a `child_process` spawning function, in every production file
 * under `ui/desktop/src`, states `windowsHide` EXPLICITLY:
 *
 *   * `windowsHide: true`  — no console window. The right answer almost always.
 *   * `windowsHide: false` — a window is the point (the user asked for a
 *     terminal). Allowed only at a site listed in `VISIBLE_BY_DESIGN`, so
 *     "silence the check" is not a thing anyone can do quietly.
 *
 * Requiring the option rather than requiring `true` is deliberate. A rule that
 * says "always hide" would be wrong at the two sites where the window IS the
 * feature, and a rule that is wrong somewhere gets an exception, and an
 * exception mechanism nobody reviews is how the flag went missing in the first
 * place. Requiring the author to *say which* costs one line and cannot be
 * satisfied by accident.
 *
 * # What it reads
 *
 * The TypeScript AST, not the text. So a `windowsHide` that appears only in a
 * comment, only in a string, or only in a NESTED object (`{ env: { windowsHide:
 * true } }`) does not count — each of those is checked by a fixture in
 * `console-window-census.test.mjs`, which also proves the analyser reports a
 * violation when there is one. A census that cannot fail is not a census.
 *
 * # What it cannot see
 *
 * Stated plainly, because a guard that oversells itself is worse than none:
 *
 *   * an options object built by a function call (`spawn(cmd, args, opts())`) —
 *     an identifier IS resolved, to a `const` in the same file, but a call is not;
 *   * a spawn reached through a variable holding the function
 *     (`const f = spawn; f(...)`);
 *   * process creation that is not `child_process` at all. `node-pty` is the one
 *     such API in the tree and has its own assertion below;
 *   * whether any of it WORKS. That is a question only Windows can answer, and
 *     `scripts/windows-console.test.mjs` asks it there — of plain Node AND of
 *     the real Electron — with a control on every claim.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

const HERE = fileURLToPath(new URL('.', import.meta.url));
export const SRC_ROOT = join(HERE, '..', 'src');

/** Module specifiers that hand out a process-spawning function. */
const CHILD_PROCESS_MODULES = new Set(['child_process', 'node:child_process']);

/** The exports of `child_process` that create a process. */
const SPAWNERS = new Set([
  'spawn',
  'spawnSync',
  'exec',
  'execSync',
  'execFile',
  'execFileSync',
  'fork',
]);

/**
 * Sites permitted to pass `windowsHide: false`, each with the reason.
 *
 * A path is matched as a suffix, and `match` must appear in the call's own
 * source text. Adding a row is a deliberate, reviewable act — which is the
 * point, since the row is the only way to opt out.
 */
export const VISIBLE_BY_DESIGN = [
  {
    file: 'src/main.ts',
    match: "spawn('cmd.exe', startArgs",
    reason:
      'cli:launch on Windows — the user clicked "open the CLI in a terminal", so a window is wanted. It is delivered by `start`, not by this option; the option records the intent. Do not read this row as "windowsHide: false makes a window appear" — inside Electron it cannot.',
  },
  {
    file: 'src/main.ts',
    match: 'spawn(term, args',
    reason:
      "cli:launch on Linux — the user's own terminal emulator, opened because they asked for it. windowsHide does not apply on Linux; it is stated so this site reads the same as its Windows twin.",
  },
];

/** Directories and filename shapes that are not shipped main-process code. */
function isProductionSource(repoRelative) {
  if (!/\.tsx?$/.test(repoRelative)) return false;
  if (/\.(test|spec)\.tsx?$/.test(repoRelative)) return false;
  const parts = repoRelative.split(sep);
  // `src/test/` is the vitest harness; `src/api/` is generated from OpenAPI.
  if (parts.includes('test') || parts.includes('api')) return false;
  return true;
}

function walk(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules' || entry === 'bin' || entry === 'web') continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) walk(full, out);
    else out.push(full);
  }
  return out;
}

/**
 * The local names in one file that refer to a process-spawning function.
 *
 * Covers: named imports (with `as` aliases), namespace/default imports used as
 * `cp.spawn`, `require('child_process')` in both destructured and namespace
 * shapes, and `promisify(execFile)` — which is how `runProbe` and the HEIC
 * converter reach `execFile`, and would otherwise be invisible.
 */
export function spawnerNames(sourceFile) {
  const direct = new Set();
  const namespaces = new Set();

  const noteBinding = (name, imported) => {
    if (SPAWNERS.has(imported)) direct.add(name);
  };

  const visit = (node) => {
    if (ts.isImportDeclaration(node) && ts.isStringLiteral(node.moduleSpecifier)) {
      if (CHILD_PROCESS_MODULES.has(node.moduleSpecifier.text)) {
        const clause = node.importClause;
        if (clause?.name) namespaces.add(clause.name.text);
        const bindings = clause?.namedBindings;
        if (bindings && ts.isNamespaceImport(bindings)) namespaces.add(bindings.name.text);
        if (bindings && ts.isNamedImports(bindings)) {
          for (const element of bindings.elements) {
            noteBinding(element.name.text, (element.propertyName ?? element.name).text);
          }
        }
      }
    }

    if (ts.isVariableDeclaration(node) && node.initializer) {
      const init = node.initializer;
      // const { spawn } = require('child_process') / const cp = require(...)
      if (
        ts.isCallExpression(init) &&
        ts.isIdentifier(init.expression) &&
        init.expression.text === 'require' &&
        init.arguments.length === 1 &&
        ts.isStringLiteral(init.arguments[0]) &&
        CHILD_PROCESS_MODULES.has(init.arguments[0].text)
      ) {
        if (ts.isIdentifier(node.name)) namespaces.add(node.name.text);
        if (ts.isObjectBindingPattern(node.name)) {
          for (const element of node.name.elements) {
            const imported = (element.propertyName ?? element.name).getText(sourceFile);
            if (ts.isIdentifier(element.name)) noteBinding(element.name.text, imported);
          }
        }
      }
      // const run = promisify(execFile)
      if (
        ts.isCallExpression(init) &&
        ts.isIdentifier(init.expression) &&
        init.expression.text === 'promisify' &&
        init.arguments.length === 1 &&
        ts.isIdentifier(node.name)
      ) {
        const inner = init.arguments[0];
        const innerName = ts.isPropertyAccessExpression(inner)
          ? inner.name.text
          : ts.isIdentifier(inner)
            ? inner.text
            : null;
        if (innerName && (direct.has(innerName) || SPAWNERS.has(innerName))) {
          direct.add(node.name.text);
        }
      }
    }

    ts.forEachChild(node, visit);
  };

  visit(sourceFile);
  return { direct, namespaces };
}

/**
 * Does this options object inherit a standard handle?
 *
 * `'inherit'` in either spelling, and any raw fd number, reach libuv as
 * UV_INHERIT_FD — which is the one thing that stops `CREATE_NO_WINDOW` being
 * applied. `'pipe'`, `'overlapped'`, `'ignore'` and a passed stream do not.
 */
function stdioOf(objectLiteral) {
  for (const property of objectLiteral.properties) {
    if (!ts.isPropertyAssignment(property)) continue;
    const key = ts.isIdentifier(property.name)
      ? property.name.text
      : ts.isStringLiteral(property.name)
        ? property.name.text
        : null;
    if (key !== 'stdio') continue;
    const value = property.initializer;
    if (ts.isStringLiteral(value)) {
      return value.text === 'inherit' ? 'inherit' : 'safe';
    }
    if (ts.isArrayLiteralExpression(value)) {
      for (const element of value.elements) {
        if (ts.isStringLiteral(element) && element.text === 'inherit') return 'inherit';
        if (ts.isNumericLiteral(element)) return 'inherit';
      }
      return 'safe';
    }
    // `stdio: [...] as ['pipe','pipe','pipe']` — read through the assertion.
    const unwrapped = ts.isAsExpression(value) ? value.expression : value;
    if (ts.isArrayLiteralExpression(unwrapped)) {
      for (const element of unwrapped.elements) {
        if (ts.isStringLiteral(element) && element.text === 'inherit') return 'inherit';
        if (ts.isNumericLiteral(element)) return 'inherit';
      }
      return 'safe';
    }
    if (ts.isStringLiteral(unwrapped)) return unwrapped.text === 'inherit' ? 'inherit' : 'safe';
    return 'unknown';
  }
  // Absent. `spawn`/`exec*` default to pipes, which is safe; `fork` defaults to
  // 'inherit' unless `silent: true`, which is not, so it is answered separately.
  return 'default';
}

/** `const NAME = { ... }` object literals, so an options identifier can be resolved. */
function objectLiteralConsts(sourceFile) {
  const found = new Map();
  const visit = (node) => {
    if (
      ts.isVariableDeclaration(node) &&
      ts.isIdentifier(node.name) &&
      node.initializer &&
      ts.isObjectLiteralExpression(node.initializer)
    ) {
      found.set(node.name.text, node.initializer);
    }
    ts.forEachChild(node, visit);
  };
  visit(sourceFile);
  return found;
}

/**
 * Read `windowsHide` off an options object literal — TOP LEVEL only.
 *
 * A property of a nested object is a different option on a different thing;
 * treating `{ env: { windowsHide: true } }` as coverage would be exactly the
 * false pass this file exists to prevent.
 */
function windowsHideOf(objectLiteral) {
  for (const property of objectLiteral.properties) {
    if (!ts.isPropertyAssignment(property)) continue;
    const key = ts.isIdentifier(property.name)
      ? property.name.text
      : ts.isStringLiteral(property.name)
        ? property.name.text
        : null;
    if (key !== 'windowsHide') continue;
    if (property.initializer.kind === ts.SyntaxKind.TrueKeyword) return true;
    if (property.initializer.kind === ts.SyntaxKind.FalseKeyword) return false;
    return 'unknown';
  }
  return null;
}

/**
 * Every spawn site in one file, with what it says about `windowsHide`.
 *
 * `state` is one of: `hidden`, `visible`, `unknown` (a non-literal value),
 * `missing` (an options object without the property), `no-options` (no options
 * argument at all), `unresolved` (options passed as something this cannot read).
 */
export function collectSpawnSites(sourceText, fileName = 'file.ts') {
  const sourceFile = ts.createSourceFile(fileName, sourceText, ts.ScriptTarget.Latest, true);
  const { direct, namespaces } = spawnerNames(sourceFile);
  const consts = objectLiteralConsts(sourceFile);
  const sites = [];

  const visit = (node) => {
    if (ts.isCallExpression(node)) {
      let callee = null;
      if (ts.isIdentifier(node.expression) && direct.has(node.expression.text)) {
        callee = node.expression.text;
      } else if (
        ts.isPropertyAccessExpression(node.expression) &&
        ts.isIdentifier(node.expression.expression) &&
        namespaces.has(node.expression.expression.text) &&
        SPAWNERS.has(node.expression.name.text)
      ) {
        callee = `${node.expression.expression.text}.${node.expression.name.text}`;
      }

      if (callee) {
        const args = node.arguments.filter(
          (a) => !ts.isArrowFunction(a) && !ts.isFunctionExpression(a)
        );
        let state = 'no-options';
        // `spawn`/`exec*` default to pipes; `fork` defaults to 'inherit' unless
        // `silent: true`, so an options-less fork is already the hazard.
        let stdio = callee.endsWith('fork') ? 'fork-default' : 'default';
        const last = args[args.length - 1];
        const readOptions = (objectLiteral) => {
          const value = windowsHideOf(objectLiteral);
          state = value === true ? 'hidden' : value === false ? 'visible' : (value ?? 'missing');
          if (value === 'unknown') state = 'unknown';
          const shape = stdioOf(objectLiteral);
          if (shape !== 'default') stdio = shape;
          else if (callee.endsWith('fork')) {
            const silent = objectLiteral.properties.some(
              (property) =>
                ts.isPropertyAssignment(property) &&
                ts.isIdentifier(property.name) &&
                property.name.text === 'silent' &&
                property.initializer.kind === ts.SyntaxKind.TrueKeyword
            );
            stdio = silent ? 'safe' : 'fork-default';
          }
        };
        if (last && ts.isObjectLiteralExpression(last)) {
          readOptions(last);
        } else if (last && ts.isIdentifier(last) && consts.has(last.text)) {
          readOptions(consts.get(last.text));
        } else if (last && couldBeOptions(last)) {
          // Something is being passed that this census cannot read. Reported as
          // its own state rather than waved through: "I could not tell" and "it
          // is fine" are different answers, and only one of them is honest.
          state = 'unresolved';
        }

        const { line } = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile));
        sites.push({ callee, line: line + 1, state, stdio, text: node.getText(sourceFile) });
      }
    }
    ts.forEachChild(node, visit);
  };

  visit(sourceFile);
  return sites;
}

/** Count `pty.spawn(` calls — node-pty is not `child_process` and needs its own rule. */
export function collectPtySites(sourceText, fileName = 'file.ts') {
  const sourceFile = ts.createSourceFile(fileName, sourceText, ts.ScriptTarget.Latest, true);
  const sites = [];
  const visit = (node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isPropertyAccessExpression(node.expression) &&
      node.expression.name.text === 'spawn' &&
      ts.isIdentifier(node.expression.expression) &&
      /^pty$/i.test(node.expression.expression.text)
    ) {
      const { line } = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile));
      sites.push({ line: line + 1 });
    }
    ts.forEachChild(node, visit);
  };
  visit(sourceFile);
  return sites;
}

/**
 * Could this argument be an options object at all?
 *
 * An array or a string in last position is the `args` of a two-argument call,
 * not options — reporting those as unreadable would bury the real finding
 * (there are no options, so `windowsHide` is Node's default of false) under a
 * vaguer one.
 */
function couldBeOptions(node) {
  return (
    ts.isIdentifier(node) ||
    ts.isCallExpression(node) ||
    ts.isPropertyAccessExpression(node) ||
    ts.isElementAccessExpression(node) ||
    ts.isAsExpression(node) ||
    ts.isSpreadElement(node) ||
    ts.isConditionalExpression(node) ||
    ts.isBinaryExpression(node) ||
    ts.isNonNullExpression(node)
  );
}

function isAllowedVisible(repoRelative, text) {
  return VISIBLE_BY_DESIGN.some(
    (row) => repoRelative.endsWith(row.file) && text.includes(row.match)
  );
}

/** Walk the tree and return `{ files, sites, ptySites, violations }`. */
export function auditTree(root = SRC_ROOT) {
  const violations = [];
  const sites = [];
  const ptySites = [];
  let files = 0;

  for (const absolute of walk(root)) {
    const repoRelative = join('src', relative(root, absolute));
    if (!isProductionSource(repoRelative)) continue;
    files += 1;
    const text = readFileSync(absolute, 'utf8');
    if (!text.includes('child_process') && !text.includes('pty.spawn')) continue;

    for (const site of collectSpawnSites(text, repoRelative)) {
      sites.push({ ...site, file: repoRelative });

      // Rule 2: inherited stdio prevents CREATE_NO_WINDOW. SW_HIDE can still
      // hide the window, but this census requires the stronger no-console path.
      if (site.stdio === 'inherit' || site.stdio === 'fork-default' || site.stdio === 'unknown') {
        violations.push({
          ...site,
          file: repoRelative,
          why:
            site.stdio === 'unknown'
              ? 'stdio is not a literal this census can read, so whether it inherits an fd is unknown'
              : site.stdio === 'fork-default'
                ? "fork() without `silent: true` defaults to stdio 'inherit', preventing CREATE_NO_WINDOW"
                : 'stdio inherits a standard handle, so CREATE_NO_WINDOW is never applied',
        });
        continue;
      }

      if (site.state === 'hidden') continue;
      if (site.state === 'visible') {
        if (isAllowedVisible(repoRelative, site.text)) continue;
        violations.push({
          ...site,
          file: repoRelative,
          why: 'passes windowsHide: false but is not listed in VISIBLE_BY_DESIGN',
        });
        continue;
      }
      violations.push({
        ...site,
        file: repoRelative,
        why:
          site.state === 'no-options'
            ? 'no options argument, so windowsHide takes its Node default of false'
            : site.state === 'missing'
              ? 'options object does not state windowsHide'
              : site.state === 'unresolved'
                ? 'options are not an object literal or a const this census can read'
                : 'windowsHide is not a boolean literal',
      });
    }

    for (const site of collectPtySites(text, repoRelative)) {
      ptySites.push({ ...site, file: repoRelative });
    }
  }

  return { files, sites, ptySites, violations };
}

export function formatViolations(violations) {
  return violations
    .map((v) => `  ${v.file}:${v.line}  ${v.callee}(…) — ${v.why}\n      ${v.text.split('\n')[0]}`)
    .join('\n');
}
