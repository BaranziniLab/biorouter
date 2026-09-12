#!/usr/bin/env node
/**
 * Mirror guard for the Biorouter design system.
 *
 * A semantic colour token is only half-real when it is declared. `--border-focus`
 * in `:root` gives you `var(--border-focus)`; it does NOT give you
 * `ring-border-focus`. For that, Tailwind needs a `--color-border-focus` entry in
 * the `@theme inline` block — and when that entry is missing the utility is not
 * generated at all, so the class silently does nothing and the element paints
 * whatever it would have painted anyway (`currentcolor` for a ring, the base
 * layer's hairline for a border). Nothing errors. Nothing looks broken in a
 * diff. jsdom cannot see it, because jsdom does not run Tailwind.
 *
 *   node scripts/check-token-mirrors.mjs
 *
 * This shipped three times — `--border-focus`, `--sidebar-ring` and
 * `--background-focus` — across nine call sites and two different stewards, one
 * of whom worked around it with `border-[var(--border-focus)]` (which renders
 * correctly, and so hid the gap instead of reporting it) while another kept
 * writing `ring-border-focus` and got `--text-default`. Both spellings look
 * right in review. Only a browser or this script can tell them apart.
 *
 * SCOPE IS DELIBERATELY NARROW, so this cannot false-positive on work in
 * flight: a token is reported only when it is (1) declared in main.css, (2)
 * actually named by a Tailwind colour utility somewhere in src/, and (3)
 * missing its `--color-*` mirror. An unused token is not a bug, and a token
 * only ever read through `var()` is not one either.
 *
 * WHERE THE MIRRORS ARE READ FROM is the one thing this check must not guess.
 * A mirror is a `--color-*` inside an `@theme inline { … }` block, found by a
 * line that BEGINS `@theme inline {` once comments are blanked out, and read
 * only up to that block's own closing brace. Until 2026-09-11 the block was
 * found with `indexOf('@theme inline')` and read to the end of the file — and
 * the first occurrence of that phrase is a comment in the plain `@theme` block,
 * far above the real one. So every `--color-*` below the comment counted: the
 * palette primitives in the plain `@theme` (`--color-coral-*`,
 * `--color-neutral-*`, …) and their remaps in the family selector blocks — 91
 * "mirrors" where the block held 61. Tailwind generates utilities from `@theme`
 * declarations and never from a selector block, so a mirror moved into
 * `:root[data-theme='alma-mater']` generated nothing — and passed.
 *
 * Exit 0: every token a utility names is mirrored. Exit 1: some are not, and
 * they are listed. Exit 2: no `@theme inline` block was found, or one never
 * closes — a refusal, not a pass. Falling back to an empty read would report
 * every token missing, which says nothing about any of them; falling back to
 * the whole file would report them all present.
 */
import { readFile, readdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join, relative } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, '..');
const CSS = join(ROOT, 'src/styles/main.css');
const SRC = join(ROOT, 'src');

/** Utility prefixes whose value comes from Tailwind's `--color-*` namespace. */
const COLOR_PREFIXES = [
  'bg',
  'text',
  'border',
  'ring',
  'inset-ring',
  'outline',
  'fill',
  'stroke',
  'decoration',
  'divide',
  'shadow',
  'accent',
  'caret',
  'from',
  'via',
  'to',
];

const css = await readFile(CSS, 'utf8');
const lineOf = (offset) => css.slice(0, offset).split('\n').length;
const at = (offset) => `${relative(ROOT, CSS)}:${lineOf(offset)}`;

/** The check cannot run. Exit 2, never 0 — see WHERE THE MIRRORS ARE READ FROM. */
function refuse(...lines) {
  console.error('CANNOT READ THE @theme inline MIRROR BLOCK\n');
  for (const line of lines) console.error(line);
  console.error(
    '\nRefusing to report OK: exit 2 means this check could not run, not that it passed.'
  );
  process.exit(2);
}

/**
 * Every `--name:` declared anywhere outside the `@theme` / `@theme inline`
 * blocks — i.e. the semantic layer a family block can re-point.
 */
const declared = new Set();
for (const m of css.matchAll(/^\s{2,}(--[a-z0-9-]+)\s*:/gm)) declared.add(m[1].slice(2));

/**
 * main.css with every comment and string blanked to spaces. Line breaks stay
 * and nothing moves, so an offset in `code` is an offset in the real file. One
 * pattern does both because a global match starts at the EARLIEST position: a
 * quote inside a comment is swallowed by the comment, and a `/*` inside a string
 * by the string. Left unterminated, each runs to where CSS ends it — the end of
 * the file for a comment, the end of the line for a string.
 */
const code = css.replace(
  /\/\*[\s\S]*?(?:\*\/|$)|"(?:[^"\\\n]|\\[\s\S])*"?|'(?:[^'\\\n]|\\[\s\S])*'?/g,
  (m) => m.replace(/[^\n]/g, ' ')
);

/** Offset of the `}` that closes the `{` at `open` in `code`, or -1 if none does. */
function closingBrace(open) {
  for (let depth = 0, i = open; i < code.length; i++) {
    if (code[i] === '{') depth++;
    else if (code[i] === '}' && --depth === 0) return i;
  }
  return -1;
}

/**
 * The `@theme inline` mirror blocks, as `{ open, close }` brace offsets. Every
 * one counts, because Tailwind reads every `@theme` block. Not `blocks()` from
 * lib/theme-tokens.mjs: that one counts braces inside comments and runs an
 * unclosed block to the end of the file — the two ways this read can silently
 * widen back into the bug described above.
 */
const mirrorBlocks = [];
for (const m of code.matchAll(/^@theme[ \t]+inline[ \t]*\{/gm)) {
  const open = m.index + m[0].length - 1;
  const close = closingBrace(open);
  if (close === -1) {
    refuse(
      `The \`@theme inline\` block opened at ${at(open)} never closes.`,
      'Reading on to the end of the file would count every `--color-*` below it as a mirror.'
    );
  }
  mirrorBlocks.push({ open, close });
}
if (mirrorBlocks.length === 0) {
  refuse(
    `No line of ${relative(ROOT, CSS)} begins \`@theme inline {\` outside a comment.`,
    'Mirrors are read from that block and nowhere else — a `--color-*` in a plain `@theme` or',
    'in a family selector block is not one — so without it there is nothing to check against.'
  );
}
const readFrom = `${relative(ROOT, CSS)}:${mirrorBlocks
  .map(({ open, close }) => `${lineOf(open)}-${lineOf(close)}`)
  .join(', ')}`;

/** Their `--color-x: var(--x)` entries. Nothing outside the braces counts. */
const mirrored = new Set();
for (const { open, close } of mirrorBlocks) {
  for (const m of code.slice(open + 1, close).matchAll(/^\s+--color-([a-z0-9-]+)\s*:/gm)) {
    mirrored.add(m[1]);
  }
}

/** Walk src/ for class strings. */
async function* files(dir) {
  for (const e of await readdir(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) {
      if (e.name === 'node_modules' || e.name.startsWith('.')) continue;
      yield* files(p);
    } else if (/\.(tsx?|css|html)$/.test(e.name)) {
      yield p;
    }
  }
}

// `ring-border-focus`, `hover:bg-overlay-hover`, `dark:text-text-muted/50`, …
const UTILITY = new RegExp(
  String.raw`(?<![\w-])(?:[a-z-]+:)*(${COLOR_PREFIXES.join('|')})-([a-z][a-z0-9-]*?)(?:\/\d+)?(?![\w/-])`,
  'g'
);

const used = new Map(); // token -> Set<file>
for await (const file of files(SRC)) {
  if (file === CSS) continue;
  const text = await readFile(file, 'utf8');
  for (const [, , rest] of text.matchAll(UTILITY)) {
    // Longest declared token that this utility could be naming. `border-focus`
    // must win over `focus` for `border-border-focus`.
    for (let name = rest; name.includes('-'); name = name.slice(name.indexOf('-') + 1)) {
      if (declared.has(name)) {
        if (!used.has(name)) used.set(name, new Set());
        used.get(name).add(relative(ROOT, file));
        break;
      }
    }
    if (declared.has(rest)) {
      if (!used.has(rest)) used.set(rest, new Set());
      used.get(rest).add(relative(ROOT, file));
    }
  }
}

const broken = [...used.keys()].filter((t) => !mirrored.has(t)).sort();

if (broken.length === 0) {
  console.log(`OK — every semantic colour token used by a utility has a @theme inline mirror`);
  console.log(
    `     (${declared.size} declared, ${mirrored.size} mirrored, ${used.size} reached from a utility)`
  );
  console.log(`     mirrors read from ${readFrom}`);
  process.exit(0);
}

console.error('MISSING @theme inline MIRRORS\n');
console.error('These tokens are declared in main.css and named by a Tailwind colour utility,');
console.error('but have no `--color-<name>` entry — so the utility is never generated and the');
console.error('call sites below silently render a fallback colour.');
console.error(`(Mirrors read from ${readFrom}.)\n`);
for (const token of broken) {
  console.error(`  --${token}`);
  console.error(`      add to @theme inline:  --color-${token}: var(--${token});`);
  for (const f of [...used.get(token)].sort()) console.error(`      used by: ${f}`);
  console.error('');
}
process.exit(1);
