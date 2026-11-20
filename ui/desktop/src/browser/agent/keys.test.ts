import { describe, expect, it } from 'vitest';
import { MODIFIER_BITS, parseChord, strokeForCharacter } from './keys';

describe('parseChord', () => {
  it('maps named keys to what CDP needs', () => {
    const enter = parseChord('Enter', 'darwin');
    expect(enter).toEqual({
      ok: true,
      stroke: { key: 'Enter', code: 'Enter', keyCode: 13, text: '\r', modifiers: 0 },
    });
    const escape = parseChord('esc', 'linux');
    expect(escape.ok && escape.stroke.key).toBe('Escape');
  });

  it('carries modifiers and the macOS editing command', () => {
    const select = parseChord('Meta+A', 'darwin');
    expect(select.ok && select.stroke).toMatchObject({
      key: 'a',
      code: 'KeyA',
      keyCode: 65,
      modifiers: MODIFIER_BITS.Meta,
      commands: ['selectAll'],
    });
    // A chord with a non-Shift modifier types no character.
    expect(select.ok && select.stroke.text).toBeUndefined();
    const linux = parseChord('Control+a', 'linux');
    expect(linux.ok && linux.stroke.commands).toEqual(['selectAll']);
    const shiftTab = parseChord('Shift+Tab', 'darwin');
    expect(shiftTab.ok && shiftTab.stroke).toMatchObject({
      key: 'Tab',
      modifiers: MODIFIER_BITS.Shift,
    });
  });

  it('refuses what is not one key', () => {
    expect(parseChord('', 'darwin').ok).toBe(false);
    expect(parseChord('Hyper+A', 'darwin').ok).toBe(false);
    expect(parseChord('Enter Enter', 'darwin').ok).toBe(false);
  });

  it('types shifted letters as capitals', () => {
    const shifted = parseChord('Shift+a', 'darwin');
    expect(shifted.ok && shifted.stroke.text).toBe('A');
  });
});

describe('strokeForCharacter', () => {
  it('types letters, capitals, newlines and characters with no key', () => {
    expect(strokeForCharacter('a')).toMatchObject({
      key: 'a',
      code: 'KeyA',
      text: 'a',
      modifiers: 0,
    });
    expect(strokeForCharacter('Q')).toMatchObject({
      key: 'Q',
      code: 'KeyQ',
      text: 'Q',
      modifiers: MODIFIER_BITS.Shift,
    });
    expect(strokeForCharacter('\n')).toMatchObject({ key: 'Enter', text: '\r' });
    expect(strokeForCharacter('é').keyCode).toBe(0);
  });
});
