import { describe, expect, it } from 'vitest';
import { deviceCodeCopy } from './copy';
import {
  caretAfterCodeCharacters,
  codeCharactersBefore,
  deviceCodeProblem,
  groupDeviceCodeInput,
  normalizeDeviceCodeInput,
} from './deviceCode';

describe('device code input', () => {
  it.each([
    ['7QK2-M9XA-3JTP-WZ4D', '7QK2M9XA3JTPWZ4D'],
    ['7qk2m9xa3jtpwz4d', '7QK2M9XA3JTPWZ4D'],
    ['7QK2 M9XA 3JTP WZ4D', '7QK2M9XA3JTPWZ4D'],
    ['  7qk2\u2013m9xa\u20143jtp wz4d\n', '7QK2M9XA3JTPWZ4D'],
    // Crockford lookalikes, exactly as the shared library reads them.
    ['IL0O-ilo0-0000-0000', '1100110000000000'],
    // Invisible characters a paste can carry are dropped.
    ['7QK2\u200bM9XA\u2060-3JTP-WZ4D', '7QK2M9XA3JTPWZ4D'],
  ])('normalizes %j', (raw, code) => {
    expect(normalizeDeviceCodeInput(raw)).toBe(code);
    expect(deviceCodeProblem(normalizeDeviceCodeInput(raw))).toBeNull();
  });

  it('refuses U rather than repairing it', () => {
    expect(normalizeDeviceCodeInput('7QK2-M9XA-3JTP-WZ4U')).toBe('7QK2M9XA3JTPWZ4U');
    expect(deviceCodeProblem('7QK2M9XA3JTPWZ4U')).toBe(deviceCodeCopy.containsU);
  });

  it('refuses a character outside the alphabet, and the wrong length', () => {
    expect(deviceCodeProblem('7QK2M9XA3JTPWZ4!')).toBe(deviceCodeCopy.invalidCharacter);
    expect(deviceCodeProblem('7QK2M9XA3JTPWZ4')).toBe(deviceCodeCopy.wrongLength);
    expect(deviceCodeProblem('7QK2M9XA3JTPWZ4DD')).toBe(deviceCodeCopy.wrongLength);
  });

  it('groups in fours for display only', () => {
    expect(groupDeviceCodeInput('7QK2M9XA3JTPWZ4D')).toBe('7QK2-M9XA-3JTP-WZ4D');
    expect(groupDeviceCodeInput('7QK2M')).toBe('7QK2-M');
    expect(groupDeviceCodeInput('')).toBe('');
  });

  it('maps the caret across regrouping', () => {
    expect(codeCharactersBefore('7QK2-M9', 7)).toBe(6);
    expect(caretAfterCodeCharacters('7QK2-M9XA', 6)).toBe(7);
    expect(caretAfterCodeCharacters('7QK2-M9XA', 4)).toBe(4);
    expect(caretAfterCodeCharacters('7QK2-M9XA', 0)).toBe(0);
  });
});
