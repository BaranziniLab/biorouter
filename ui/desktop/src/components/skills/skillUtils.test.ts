import { describe, it, expect } from 'vitest';
import { parseSkillFrontmatter, toSlug, withoutStagingNonce } from './skillUtils';

describe('parseSkillFrontmatter', () => {
  it('returns name and description from valid frontmatter', () => {
    const content = `---\nname: my-skill\ndescription: A test skill\n---\nBody here`;
    expect(parseSkillFrontmatter(content)).toEqual({
      name: 'my-skill',
      description: 'A test skill',
    });
  });

  it('returns null when frontmatter is missing', () => {
    expect(parseSkillFrontmatter('# No frontmatter')).toBeNull();
  });

  it('returns null when name is missing', () => {
    const content = `---\ndescription: only desc\n---\nBody`;
    expect(parseSkillFrontmatter(content)).toBeNull();
  });

  it('ignores extra frontmatter fields (user-invocable, hooks)', () => {
    const content = `---\nname: ralph\ndescription: Test\nuser-invocable: true\n---\nBody`;
    expect(parseSkillFrontmatter(content)).toEqual({ name: 'ralph', description: 'Test' });
  });

  it('strips surrounding quotes from an inline description', () => {
    const content = `---\nname: q\ndescription: "Quoted desc with: colon"\n---\nBody`;
    expect(parseSkillFrontmatter(content)).toEqual({
      name: 'q',
      description: 'Quoted desc with: colon',
    });
  });

  it('folds a `>-` block-scalar description into spaced text', () => {
    const content = [
      '---',
      'name: update-soul',
      'description: >-',
      "  Update the user's personal Soul knowledge base from their",
      '  conversation history. Load this skill when running a Meditation.',
      'skills:',
      '- update-soul',
      '---',
      'Body',
    ].join('\n');
    expect(parseSkillFrontmatter(content)).toEqual({
      name: 'update-soul',
      description:
        "Update the user's personal Soul knowledge base from their conversation history. Load this skill when running a Meditation.",
    });
  });

  it('keeps newlines for a `|` literal block scalar', () => {
    const content = `---\nname: lit\ndescription: |\n  line one\n  line two\n---\nBody`;
    expect(parseSkillFrontmatter(content)).toEqual({
      name: 'lit',
      description: 'line one\nline two',
    });
  });

  it('returns null when a block-scalar description is empty', () => {
    const content = `---\nname: empty\ndescription: >-\n---\nBody`;
    expect(parseSkillFrontmatter(content)).toBeNull();
  });
});

describe('toSlug', () => {
  it('lowercases and replaces special chars with hyphens', () => {
    expect(toSlug('My Skill!')).toBe('my-skill');
  });

  it('strips .md extension', () => {
    expect(toSlug('my-skill.md')).toBe('my-skill');
  });

  it('collapses multiple hyphens', () => {
    expect(toSlug('a  b')).toBe('a-b');
  });
});

/**
 * The nonce `registry:download` used to prepend to a staged archive's filename,
 * and which the daemon's importer then read as the package id for any archive
 * declaring no name of its own.
 */
describe('withoutStagingNonce', () => {
  it('recognises the twelve-hex-digit staging nonce', () => {
    expect(withoutStagingNonce('d92c1c985d54-single-cell')).toBe('single-cell');
    expect(withoutStagingNonce('3fdc9c5f1b82-single-cell')).toBe('single-cell');
    // The whole remainder survives, dashes and all.
    expect(withoutStagingNonce('a1b2c3d4e5f6-hi-c-analysis')).toBe('hi-c-analysis');
    // `biorouter serve` wrote a 16-digit nanosecond stamp instead.
    expect(withoutStagingNonce('18d4c57a2b75d100-single-cell')).toBe('single-cell');
  });

  it('reports no nonce for a name that never carried one', () => {
    expect(withoutStagingNonce('single-cell')).toBeNull();
    expect(withoutStagingNonce('hi-c-analysis')).toBeNull();
    expect(withoutStagingNonce('scientific-research')).toBeNull();
  });

  /**
   * The rule is "exactly the shape `crypto.randomBytes(6).toString('hex')`
   * produces", not "anything before a dash" — which would have turned
   * `chip-single-cell` into a false match for the registry's `single-cell`.
   */
  it('refuses every near miss', () => {
    expect(withoutStagingNonce('chip-single-cell')).toBeNull();
    // Neither writer produces eleven, thirteen, fifteen or seventeen digits.
    expect(withoutStagingNonce('d92c1c985d5-single-cell')).toBeNull();
    expect(withoutStagingNonce('d92c1c985d54a-single-cell')).toBeNull();
    expect(withoutStagingNonce('18d4c57a2b75d10-single-cell')).toBeNull();
    expect(withoutStagingNonce('18d4c57a2b75d1000-single-cell')).toBeNull();
    // Uppercase hex is not what `toString('hex')` writes.
    expect(withoutStagingNonce('D92C1C985D54-single-cell')).toBeNull();
    // A non-hex letter in the run.
    expect(withoutStagingNonce('d92c1c985dz4-single-cell')).toBeNull();
    // A nonce with nothing after it is not a package name.
    expect(withoutStagingNonce('d92c1c985d54-')).toBeNull();
    // A year is not a nonce.
    expect(withoutStagingNonce('2024-cohort')).toBeNull();
  });
});
