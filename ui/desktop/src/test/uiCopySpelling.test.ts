import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';
import { extractUserVisibleStrings, readsAsProse, type CopyString } from './userVisibleCopy';
import { ACCEPTED_CONVENTION, REJECTED_FORMS, VARIANT_PAIRS } from './spellingVariants';

/**
 * One spelling convention for the product's user-visible copy, enforced at the
 * source.
 *
 * **The convention: American English.** Decided 2026-09-09 by measuring every
 * user-visible string in the product rather than by grepping the tree — see
 * `userVisibleCopy.ts` for why a grep cannot answer this, and
 * `docs/desktop-ui/settings-visual-vocabulary.md` rule 11 for the rule as
 * prose. What the measurement found:
 *
 * | surface | US | British |
 * |---|---|---|
 * | desktop renderer, user-visible strings | 11 | 9 |
 * | landing site (biorouter.ucsf.edu), visible text | 86 | 6 |
 * | CLI string literals | 76 | 26 |
 * | shipped skill text (`builtin_skills/`) | 8 | 9 |
 *
 * **The trade-off, recorded because it is real.** The handoff that asked for
 * this expected British, on the grounds that "the product's own copy uses
 * British spellings elsewhere". Measured, that is not true of the product's
 * user-visible copy: the website is 14:1 American and the renderer already
 * leans American. British forms are concentrated in the repository's
 * CONTRIBUTOR prose — `CLAUDE.md`, `docs/`, code comments — which this
 * convention deliberately does not govern, and in one product name. So the
 * repository stays bilingual, and the line is drawn where a reader changes:
 * copy a user reads is American, prose a contributor reads is left alone.
 *
 * Two further reasons the American column was the one to keep:
 *
 * - **The identifiers cannot move.** `color`, `center`, `dialog`, `catalog`,
 *   `license`, `artifact`, `initialize` are CSS, DOM, API and product
 *   identifiers. A British copy convention would put every label permanently at
 *   odds with the symbol beside it, and that seam had already split this
 *   codebase: the marketplace said "Marketplace catalogue" in one component and
 *   "Loading catalog…" in two others.
 * - **UCSF and this repository's operator write American English**, and a
 *   convention nobody writes by hand is one that rots — the argument
 *   `CLAUDE.md` already makes about a CI gate that fails on arrival.
 *
 * **Auto Visualiser stays British**, along with Biorouter, BAAM and every
 * provider name: a product name is a proper noun, not a spelling.
 */
const SRC = join(__dirname, '..');
const REPO = join(SRC, '..', '..', '..');

/**
 * Every tree whose copy this convention governs, each with its own exclusions —
 * the shape `settingsVocabulary.test.ts` established, for the same reason:
 * adding a surface is one line, and each surface states its own scope.
 */
const ROOTS: { dir: string; outOfScope: string[]; extensions: string[] }[] = [
  {
    dir: SRC,
    // `api/` is generated from the OpenAPI spec and hand-editing it is
    // forbidden; `bin/` and `web/` are build outputs; `test/` is this guard's
    // own machinery, where the rejected forms appear as DATA.
    outOfScope: ['api/', 'bin/', 'web/', 'test/'],
    extensions: ['.ts', '.tsx'],
  },
  // The shipped skill text. It is prose a model reads rather than JSX, so it is
  // scanned line by line — and it is guarded from here rather than from Rust
  // because there is no copy guard on that side and this is the only one.
  {
    dir: join(REPO, 'crates', 'biorouter', 'src', 'agents', 'builtin_skills'),
    outOfScope: [],
    extensions: ['.md'],
  },
];

/**
 * Phrases that keep a rejected spelling, each with the reason it is exempt.
 *
 * ⚠ This is for PROPER NOUNS and wire values, not for copy someone would rather
 * not change. A new entry here is a claim that the words are a name; a sentence
 * that merely reads better in British English is not one.
 */
const ALLOWED_PHRASES: { phrase: string; because: string }[] = [
  {
    phrase: 'Auto Visualiser',
    because:
      "the built-in server's product name, shown in the capability list and the @-mention picker. " +
      'Matched case-insensitively: the crate table in `develop-biorouter/SKILL.md` lists the ' +
      'built-in servers in lower case, and the name is the name in either case.',
  },
];

function sourceFiles(): { path: string; rel: string; text: string; markdown: boolean }[] {
  const found: { path: string; rel: string; text: string; markdown: boolean }[] = [];
  for (const { dir: root, outOfScope, extensions } of ROOTS) {
    const walk = (dir: string) => {
      for (const entry of readdirSync(dir)) {
        const path = join(dir, entry);
        if (statSync(path).isDirectory()) {
          if (!outOfScope.some((prefix) => `${relative(root, path)}/`.startsWith(prefix)))
            walk(path);
          continue;
        }
        if (!extensions.some((extension) => entry.endsWith(extension))) continue;
        if (entry.includes('.test.') || entry.endsWith('.d.ts')) continue;
        if (outOfScope.some((prefix) => relative(root, path).startsWith(prefix))) continue;
        found.push({
          path,
          rel: relative(REPO, path),
          text: readFileSync(path, 'utf8'),
          markdown: entry.endsWith('.md'),
        });
      }
    };
    walk(root);
  }
  return found;
}

const FILES = sourceFiles();

/** Strip the exempt proper nouns before scanning, so the name is invisible to the rule. */
function withoutAllowedPhrases(text: string): string {
  let stripped = text;
  for (const { phrase } of ALLOWED_PHRASES) {
    stripped = stripped.replace(
      new RegExp(phrase.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'gi'),
      ' '
    );
  }
  return stripped;
}

export interface SpellingOffence {
  file: string;
  line: number;
  found: string;
  write: string;
  text: string;
}

function offences(): SpellingOffence[] {
  const strings: CopyString[] = [];
  for (const { rel, text, markdown } of FILES) {
    if (markdown) {
      // Markdown is prose end to end. Fenced code and inline code are not, and
      // an identifier in backticks is exactly the thing that must not be
      // re-spelled, so both are removed before the lines are read.
      //
      // ⚠ A stated gap, not an oversight: a fence with no language tag can hold
      // prose — `develop-biorouter/SKILL.md`'s change checklist does, and its
      // "Behaviour implemented in the biorouter crate" line was swept BY HAND
      // because this loop cannot see it. A regression inside an untagged fence
      // will not fail this test. The alternative — reading untagged fences as
      // prose — would put shell transcripts and JSON under a spelling rule,
      // which is the more damaging way to be wrong.
      let fenced = false;
      text.split('\n').forEach((line, index) => {
        if (/^\s*```/.test(line)) {
          fenced = !fenced;
          return;
        }
        if (fenced) return;
        const prose = line.replace(/`[^`]*`/g, ' ');
        if (prose.trim())
          strings.push({
            file: rel,
            line: index + 1,
            text: prose,
            carrier: 'markdown',
            tier: 'prose',
          });
      });
      continue;
    }
    strings.push(...extractUserVisibleStrings(text, rel));
  }

  const found: SpellingOffence[] = [];
  for (const string of strings) {
    const haystack = withoutAllowedPhrases(string.text);
    for (const [rejected, write] of REJECTED_FORMS) {
      if (new RegExp(`(?<![A-Za-z])${rejected}(?![A-Za-z])`, 'i').test(haystack)) {
        found.push({
          file: string.file,
          line: string.line,
          found: rejected,
          write,
          text: string.text.replace(/\s+/g, ' ').slice(0, 100),
        });
      }
    }
  }
  return found;
}

describe(`user-visible copy is ${ACCEPTED_CONVENTION} English`, () => {
  /**
   * A walker that silently matched nothing would make the rule below vacuous —
   * and that is not hypothetical here, because one root is reached by a
   * relative path OUT of `ui/desktop` and would resolve to nothing if this file
   * ever moved. Both roots are asserted separately for that reason.
   */
  it('finds the sources it claims to govern', () => {
    for (const { dir } of ROOTS) {
      expect(FILES.filter(({ path }) => path.startsWith(dir)).length).toBeGreaterThan(0);
    }
    expect(FILES.length).toBeGreaterThan(400);
    const covered = new Set(FILES.map(({ rel }) => rel));
    expect(covered).toContain('ui/desktop/src/components/settings/SettingsView.tsx');
    expect(covered).toContain(
      'crates/biorouter/src/agents/builtin_skills/about-biorouter/SKILL.md'
    );
  });

  /**
   * The extractor is what makes the rule mean anything, so its two tiers are
   * pinned here rather than left to be inferred from a passing sweep.
   */
  it('reads copy out of the shapes this app actually writes it in', () => {
    const source = `
      export const Example = ({ busy }: { busy: boolean }) => (
        <Dialog>
          <p className="text-center items-center">Visible sentence</p>
          <DialogDescription>{busy ? 'Working on it now' : 'Idle right now'}</DialogDescription>
          <Button aria-label="Close panel" />
        </Dialog>
      );
      const note = { note: 'A sentence shown to the user.', reason: 'cancelled' };
      console.error('A developer log line nobody reads');
      throw new Error('A developer error nobody reads');
    `;
    const texts = extractUserVisibleStrings(source, 'Example.tsx').map(({ text }) => text);
    expect(texts).toContain('Visible sentence');
    expect(texts).toContain('Working on it now');
    expect(texts).toContain('Idle right now');
    expect(texts).toContain('Close panel');
    expect(texts).toContain('A sentence shown to the user.');
    // The class list, the wire value, the log and the thrown error are not copy.
    expect(texts).not.toContain('text-center items-center');
    expect(texts).not.toContain('cancelled');
    expect(texts).not.toContain('A developer log line nobody reads');
    expect(texts).not.toContain('A developer error nobody reads');
  });

  it('rejects a variant only when it is a whole word', () => {
    expect(readsAsProse('Colour the row on hover')).toBe(true);
    expect(readsAsProse('flex items-center gap-2')).toBe(false);
  });

  /** The rule. */
  it('has no rejected spelling in any user-visible string', () => {
    expect(
      offences().map(
        ({ file, line, found, write, text }) => `${file}:${line} "${found}" → "${write}" — ${text}`
      )
    ).toEqual([]);
  });

  /**
   * The allow-list has to be doing work: `visualiser` IS a rejected form, and
   * the only reason "Auto Visualiser" survives the sweep is the exemption. If
   * the pair list ever stopped generating that word, this exemption would go
   * silently inert and a stray "visualiser" in ordinary copy would ship.
   */
  it('keeps the product name that the convention would otherwise reject', () => {
    expect(REJECTED_FORMS.has('visualiser')).toBe(true);
    for (const { phrase } of ALLOWED_PHRASES) {
      expect(
        [...REJECTED_FORMS.keys()].some((word) =>
          new RegExp(`(?<![A-Za-z])${word}(?![A-Za-z])`, 'i').test(phrase)
        )
      ).toBe(true);
    }
    expect(
      offences().filter(({ text }) => ALLOWED_PHRASES.some(({ phrase }) => text.includes(phrase)))
    ).toEqual([]);
  });

  it('maps every pair in one direction, with no word rejected and accepted at once', () => {
    expect(VARIANT_PAIRS.length).toBeGreaterThan(150);
    const accepted = new Set(
      VARIANT_PAIRS.map(([american, british]) =>
        ACCEPTED_CONVENTION === 'american' ? american : british
      )
    );
    expect([...REJECTED_FORMS.keys()].filter((word) => accepted.has(word))).toEqual([]);
  });
});
