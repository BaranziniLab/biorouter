import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  affiliationExplanation,
  affiliationPresentation,
  institutionLabel,
  readProviderAffiliation,
  type ProviderAffiliation,
} from './providerAffiliation';

/** A `ProviderDetails` row as `GET /config/providers` serves it. */
const row = (affiliation: unknown) => ({
  name: 'versa_azure',
  metadata: { name: 'versa_azure', tier: 'private' },
  is_configured: true,
  provider_type: 'Preferred',
  affiliation,
});

describe('readProviderAffiliation', () => {
  /**
   * ⚠ The state the brief calls out third: **a public model has no affiliation
   * at all**, and the daemon says so by omitting the field. Showing an empty
   * chip for it would put a constraint-shaped ornament on the one kind of model
   * that has none of the private tier's protections.
   */
  it('reads no affiliation for a public provider', () => {
    expect(readProviderAffiliation(row(null))).toBeNull();
    expect(readProviderAffiliation(row(undefined))).toBeNull();
    // ...and for a row that predates the field entirely.
    expect(
      readProviderAffiliation({ name: 'openai', metadata: {}, is_configured: true })
    ).toBeNull();
    // ...and for a row that is not a row.
    expect(readProviderAffiliation(undefined)).toBeNull();
    expect(readProviderAffiliation('versa_azure')).toBeNull();
  });

  it('reads an institution with its published name and its raw id', () => {
    const parsed = readProviderAffiliation(
      row({ kind: 'institutions', institutions: [{ id: 'ucsf', display_name: 'UCSF' }] })
    );
    expect(parsed).toEqual({
      kind: 'institutions',
      institutions: [{ id: 'ucsf', display_name: 'UCSF' }],
    });
  });

  /**
   * Task 47: an institution the registry does not publish is a *mismatch* whose
   * raw id is surfaced. Dropping the row for want of a pretty name would make a
   * real constraint disappear from the only surface the user can see it on.
   */
  it('keeps an institution the registry publishes no name for', () => {
    const parsed = readProviderAffiliation(
      row({ kind: 'institutions', institutions: [{ id: 'some-lab' }] })
    );
    expect(parsed?.institutions).toEqual([{ id: 'some-lab', display_name: null }]);
    expect(institutionLabel({ id: 'some-lab', display_name: null })).toBe('some-lab');
  });

  /**
   * ⚠ **An unrecognised kind is `null`, never a default member.** A fourth
   * affiliation the daemon learns before this renderer does must render as
   * nothing — `local` in particular is the MOST permissive value in DR-26's
   * model, so falling back to it would put the blanket-permission chip on a
   * model that never claimed it.
   */
  it('refuses an affiliation it does not recognise rather than defaulting one', () => {
    expect(readProviderAffiliation(row({ kind: 'consortium', institutions: [] }))).toBeNull();
    expect(readProviderAffiliation(row({ institutions: [{ id: 'ucsf' }] }))).toBeNull();
  });

  /**
   * The daemon cannot send this (`InstitutionSet` refuses to be empty, because
   * an empty model set is a subset of every allowlist and would therefore reach
   * every private extension), so this only fires on a mangled payload — and an
   * institution chip naming nobody would state a covering agreement that covers
   * no one.
   */
  it('refuses an institutions affiliation that names nobody', () => {
    expect(readProviderAffiliation(row({ kind: 'institutions', institutions: [] }))).toBeNull();
    expect(
      readProviderAffiliation(row({ kind: 'institutions', institutions: [{ id: '' }, 7] }))
    ).toBeNull();
  });

  it('reads local and unstated, which carry no institutions', () => {
    expect(readProviderAffiliation(row({ kind: 'local', institutions: [] }))).toEqual({
      kind: 'local',
      institutions: [],
    });
    expect(readProviderAffiliation(row({ kind: 'unstated' }))).toEqual({
      kind: 'unstated',
      institutions: [],
    });
  });
});

describe('affiliationPresentation', () => {
  const of = (a: ProviderAffiliation | null) => affiliationPresentation(a);

  it('says nothing when there is no affiliation', () => {
    expect(of(null)).toBeNull();
    expect(affiliationPresentation(undefined)).toBeNull();
  });

  /**
   * ⚠ **The inversion this task exists to prevent.** `local` is the *most*
   * permissive affiliation — it reaches every private extension, because no
   * transfer occurs at all — so a user seeing it beside `UCSF` must not read it
   * as more restricted. Two things hold that here and both are asserted: it does
   * not render as an institution-shaped name, and its sentence states the reach
   * outright.
   */
  it('renders local as the least restricted affiliation, not a narrower institution', () => {
    const words = of({ kind: 'local', institutions: [] })!;
    expect(words.label).toBe('On this machine');
    // The line at rest says where the work happens; its InfoTip states the reach.
    expect(words.title).toBe('Runs on this machine, so nothing leaves it.');
    const explanation = affiliationExplanation(words);
    expect(explanation).toContain('least restricted');
    expect(explanation).toContain('every private extension');
    // Not phrased as an institution: no possessive agreements clause, which is
    // what the institutions arm uses.
    expect(explanation).not.toContain('Covered by');
  });

  /**
   * ⚠ **The second state a naive mapping renders wrong.** A private model naming
   * no institution clears ONLY unconstrained extensions — the least-reaching
   * private model there is — so an empty-looking chip would show it as the least
   * constrained one.
   */
  it('renders unstated as a stated fact, not as an absence', () => {
    const words = of({ kind: 'unstated', institutions: [] });
    expect(words?.label).toBe('No stated institution');
    expect(words?.label.trim().length).toBeGreaterThan(0);
    expect(words?.title).toContain('does not state whose agreements cover it');
    // ...and it is emphatically not the local sentence.
    expect(words?.title).not.toContain('least restricted');
  });

  it('names every institution a spanning model is covered by', () => {
    const words = of({
      kind: 'institutions',
      institutions: [
        { id: 'stanford', display_name: 'Stanford' },
        { id: 'ucsf', display_name: 'UCSF' },
      ],
    });
    // Both, never a representative: picking one would tell the user the chat is
    // covered by an institution it only half is.
    expect(words?.label).toBe('Stanford and UCSF');
    expect(words?.title).toContain('Stanford and UCSF');
  });

  it('states that compliance does not transfer between institutions', () => {
    const words = of({
      kind: 'institutions',
      institutions: [{ id: 'ucsf', display_name: 'UCSF' }],
    });
    expect(words?.label).toBe('UCSF');
    // One line at rest; the cross-institutional approval and the compliance sentence are its
    // InfoTip (owner rule "no walls of text", Phase C ruling F-13).
    expect(words?.title).toBe('Covered by UCSF’s agreements.');
    expect(words?.detail).toContain('Compliance does not transfer');
    expect(words?.detail).toContain('needs your approval first');
  });

  /** Owner rule "no walls of text" (Phase C ruling F-13): one sentence at rest, at most two more. */
  it('keeps each line to one sentence and its detail to two', () => {
    for (const kind of ['local', 'institutions', 'unstated'] as const) {
      const words = of({
        kind,
        institutions: kind === 'institutions' ? [{ id: 'ucsf', display_name: 'UCSF' }] : [],
      })!;
      const sentences = (text: string) => text.split(/(?<=\.)\s+/).filter(Boolean).length;
      expect(sentences(words.title), kind).toBe(1);
      expect(sentences(words.detail), kind).toBeLessThanOrEqual(2);
    }
  });

  /** Every kind has words. A `Record` keyed on the union makes this structural. */
  it('has a rendering for every affiliation it will accept', () => {
    for (const kind of ['local', 'institutions', 'unstated'] as const) {
      const parsed = readProviderAffiliation(
        row({ kind, institutions: kind === 'institutions' ? [{ id: 'ucsf' }] : [] })
      );
      expect(parsed, `${kind} must parse`).not.toBeNull();
      expect(affiliationPresentation(parsed), `${kind} must have words`).not.toBeNull();
    }
  });
});

/**
 * ⚠ **The affiliation is rendered and NEVER sent** (issue #56, DR-26).
 *
 * `CrossAffiliationGrantRequest` deliberately carries no affiliation: the
 * daemon reads the institution from the same sample that produced the warning,
 * so a client-supplied one would let a caller record an acceptance for a triple
 * the user was never shown. Now that the renderer *holds* an affiliation, the
 * shortest path to that bug is a well-meaning edit that passes the badge's value
 * into the grant call, and nothing else in the tree would notice.
 *
 * The sibling audit in `utils/crossAffiliation.test.ts` pins which module may
 * name the grant call; this one pins the other direction — that no module which
 * reads an affiliation is that module.
 */
describe('the rendered affiliation is never sent back to the daemon', () => {
  const srcRoot = join(process.cwd(), 'src');
  const isProduction = (name: string) => /\.(ts|tsx)$/.test(name) && !/\.(test|spec)\./.test(name);

  const walk = (dir: string): string[] =>
    readdirSync(dir).flatMap((entry) => {
      const full = join(dir, entry);
      if (statSync(full).isDirectory()) return entry === 'node_modules' ? [] : walk(full);
      return isProduction(entry) ? [full] : [];
    });

  it('is not named by any module that reads one', () => {
    const files = walk(srcRoot);
    // A broken walk reports the same empty set as a clean tree.
    expect(files.length).toBeGreaterThan(400);

    // Composed, so this test file does not match its own audit.
    const grantCall = ['agent', 'CrossAffiliationGrant'].join('');
    const readers = files.filter((file) => {
      const source = readFileSync(file, 'utf8');
      return source.includes('providerAffiliation') || source.includes('useBoundAffiliation');
    });
    // The audit is worthless if it found nothing to audit.
    expect(readers.length).toBeGreaterThanOrEqual(4);

    const offenders = readers
      .filter((file) => readFileSync(file, 'utf8').includes(grantCall))
      .map((file) => relative(srcRoot, file).split(sep).join('/'))
      .sort();

    expect(offenders).toEqual([]);
  });
});
