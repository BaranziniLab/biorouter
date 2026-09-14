import { describe, expect, it } from 'vitest';
import {
  failedToast,
  installedToast,
  installProgressLabel,
  NAMED_INSTALLS,
  registrySkillCount,
} from './installCopy';
import { FALLBACK_REGISTRY } from './registry';

describe('registrySkillCount', () => {
  it.each([
    ['3 skills · auto-applied', 3],
    ['14 skills · auto-applied', 14],
    ['13 skills · TDD, debugging, planning, more', 13],
    ['1 skill · auto-applied', 1],
    ['User-invocable · /scientific-research', 1],
    ['Auto-applied · R plotting', 1],
    ['', 1],
  ])('reads %j as %i', (type, expected) => {
    expect(registrySkillCount({ type })).toBe(expected);
  });

  /// The registry has no count field, so the count rides a phrase written for
  /// people. Pinned against the bundled registry: every row that mentions a
  /// number of skills anywhere must lead with it, or the parse would call a
  /// package one skill and the button would under-promise again.
  it('reads every package in the bundled registry', () => {
    const packages = FALLBACK_REGISTRY.skills.filter((s) => /\d+\s+skills?/i.test(s.type));
    expect(packages.length).toBeGreaterThan(0);
    for (const row of packages) {
      const stated = Number(/(\d+)\s+skills?/i.exec(row.type)![1]);
      expect({ id: row.id, count: registrySkillCount(row) }).toEqual({ id: row.id, count: stated });
    }
    const primer = FALLBACK_REGISTRY.skills.find((s) => s.id === 'primer-design');
    expect(primer && registrySkillCount(primer)).toBe(3);
  });
});

describe('installedToast', () => {
  it('counts skills and names a package with its count', () => {
    expect(installedToast([{ name: 'primer-design', skills: 3, isPackage: true }])).toEqual({
      title: '3 skills installed',
      msg: 'Added to Biorouter Skills: primer-design (3 skills)',
    });
  });

  it('names a single skill without a count', () => {
    expect(installedToast([{ name: 'r-scripting', skills: 1, isPackage: false }])).toEqual({
      title: '1 skill installed',
      msg: 'Added to Biorouter Skills: r-scripting',
    });
  });

  it('adds a package and a skill together — the measured "2 skills" for 15', () => {
    expect(
      installedToast([
        { name: 'single-cell', skills: 14, isPackage: true },
        { name: 'r-scripting', skills: 1, isPackage: false },
      ])
    ).toEqual({
      title: '15 skills installed',
      msg: 'Added to Biorouter Skills: single-cell (14 skills) and r-scripting',
    });
  });

  it(`names ${NAMED_INSTALLS} installs, then says how many more`, () => {
    const many = ['a', 'b', 'c', 'd', 'e'].map((name) => ({ name, skills: 2, isPackage: true }));
    expect(installedToast(many)).toEqual({
      title: '10 skills installed',
      msg: 'Added to Biorouter Skills: a (2 skills), b (2 skills), c (2 skills) and 2 more',
    });
    expect(installedToast(many.slice(0, 3)).msg).toBe(
      'Added to Biorouter Skills: a (2 skills), b (2 skills) and c (2 skills)'
    );
  });
});

describe('installedToast — a reinstall is not an install', () => {
  /// The two-window case: window B installed Clinical Biostatistics while
  /// window A's dialog still offered it, and A's install replaced it and said
  /// "12 skills installed".
  it('says reinstalled when every unit replaced an install', () => {
    expect(
      installedToast([
        { name: 'clinical-biostatistics', skills: 12, isPackage: true, replaced: true },
      ])
    ).toEqual({
      title: '12 skills reinstalled',
      msg: 'Replaced in Biorouter Skills: clinical-biostatistics (12 skills)',
    });
  });

  it('keeps the two counts apart when a run did both', () => {
    expect(
      installedToast([
        { name: 'alignment', skills: 7, isPackage: true },
        { name: 'clinical-biostatistics', skills: 12, isPackage: true, replaced: true },
      ])
    ).toEqual({
      title: '7 skills installed, 12 reinstalled',
      msg: 'Added to Biorouter Skills: alignment (7 skills). Replaced: clinical-biostatistics (12 skills)',
    });
  });
});

describe('failedToast', () => {
  it('names the one row that failed', () => {
    expect(failedToast([{ name: 'Primer Design', error: 'network down' }])).toEqual({
      title: 'Primer Design was not installed',
      msg: 'network down',
    });
  });

  it('counts failed selections, never their skills', () => {
    expect(
      failedToast([
        { name: 'Single-cell', error: 'disk full' },
        { name: 'Primer Design', error: 'disk full' },
        { name: 'R Scripting', error: 'disk full' },
      ])
    ).toEqual({
      title: '3 selections were not installed',
      msg: 'Single-cell: disk full (and 2 more)',
    });
  });
});

describe('installProgressLabel', () => {
  it('names the download in flight and where it is in the queue', () => {
    expect(installProgressLabel('Single-cell', 1, 2)).toBe('Installing Single-cell (1 of 2)…');
    expect(installProgressLabel('Primer Design', 1, 1)).toBe('Installing Primer Design…');
  });
});
