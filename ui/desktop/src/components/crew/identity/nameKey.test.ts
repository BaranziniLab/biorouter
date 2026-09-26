import { describe, expect, it } from 'vitest';
import { cleanName, displayNameKey, isMachineIdShaped, nameKey, stripIgnorable } from './nameKey';

/**
 * The examples are the naming design's own ("Normalization and keys"), so this
 * port and `biorouter_crew::name_key` are held to the same table.
 */
describe('nameKey', () => {
  it.each([
    'Analysis Lab',
    'analysis-lab',
    'ANALYSIS_LAB',
    'Analysis.Lab',
    'Ａｎａｌｙｓｉｓ Ｌａｂ',
    'Analysis Lab\uFE0F',
  ])('keys %j as analysis-lab', (name) => {
    expect(nameKey(name)).toBe('analysis-lab');
  });

  it('does not let a variation selector or zero-width character make a lookalike', () => {
    expect(nameKey('Anal\u200Bysis Lab')).toBe('analysis-lab');
    expect(nameKey('Analysis\u200D Lab')).toBe('analysis-lab');
    expect(nameKey('\uFEFFAnalysis Lab')).toBe('analysis-lab');
  });

  it('collapses separator runs and trims them from both ends', () => {
    expect(nameKey('--Analysis  .._Lab--')).toBe('analysis-lab');
    expect(nameKey(' methods ')).toBe('methods');
  });

  it('uses the default lowercase mapping, not full case folding, so ß and ss stay distinct', () => {
    expect(nameKey('Straße')).toBe('straße');
    expect(nameKey('Strasse')).toBe('strasse');
    expect(nameKey('Straße')).not.toBe(nameKey('Strasse'));
  });

  it('keeps names in other scripts', () => {
    expect(nameKey('李明 Li Ming')).toBe('李明-li-ming');
    expect(nameKey('Sam Park')).toBe(nameKey('sam park'));
  });

  it('does not attempt the confusable skeleton, which is the daemon’s job (S2b)', () => {
    expect(nameKey('anaIysis')).not.toBe(nameKey('analysis'));
  });
});

describe('cleanName', () => {
  it('normalizes to NFC, trims White_Space and collapses runs to one space', () => {
    expect(cleanName('  Bob\t\u00A0 Lee \n')).toBe('Bob Lee');
    expect(cleanName('Cafe\u0301')).toBe('Café');
  });

  it('trims White_Space only, not U+FEFF, exactly as Rust’s trim does', () => {
    expect(cleanName('\uFEFFBob')).toBe('\uFEFFBob');
  });
});

describe('displayNameKey', () => {
  it('cleans before keying, so a tab separates words like a space', () => {
    expect(displayNameKey('Sam\tPark')).toBe(displayNameKey('Sam Park'));
  });
});

describe('stripIgnorable', () => {
  it('removes Default_Ignorable_Code_Point characters and nothing else', () => {
    expect(stripIgnorable('a\u200Bb\u200Dc\uFE0F\u3164d')).toBe('abcd');
    expect(stripIgnorable('Bob Lee')).toBe('Bob Lee');
  });
});

describe('isMachineIdShaped', () => {
  it.each([
    '3f2a9c1e-77b0-4d4e-9a1b-2c3d4e5f6a7b',
    '3F2A9C1E-77B0-4D4E-9A1B-2C3D4E5F6A7B',
    '3f2a9c1e77b04d4e9a1b2c3d4e5f6a7b',
    '{3f2a9c1e-77b0-4d4e-9a1b-2c3d4e5f6a7b}',
    'urn:uuid:3f2a9c1e-77b0-4d4e-9a1b-2c3d4e5f6a7b',
    'a'.repeat(64),
    ' 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef ',
  ])('reads %j as a machine ID', (value) => {
    expect(isMachineIdShaped(value)).toBe(true);
  });

  it.each(['Analysis Lab', 'cafe', 'deadbeef', 'a'.repeat(63), 'methods', '1001'])(
    'does not read %j as a machine ID',
    (value) => {
      expect(isMachineIdShaped(value)).toBe(false);
    }
  );
});
