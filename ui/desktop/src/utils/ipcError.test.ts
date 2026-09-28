import { describe, expect, it } from 'vitest';
import { failureSentence, ipcErrorMessage, unwrapIpcError } from './ipcError';

const WRAPPED =
  "Error invoking remote method 'crew:select-transfer-file': Error: Finish the open Save or Open window first.";

describe('ipcErrorMessage (FILES-F6)', () => {
  it("takes Electron's invoke wrapper off, once", () => {
    expect(ipcErrorMessage(WRAPPED)).toBe('Finish the open Save or Open window first.');
    expect(
      ipcErrorMessage("Error invoking remote method 'x': TypeError: Cannot read a property")
    ).toBe('Cannot read a property');
    expect(ipcErrorMessage("Error invoking remote method 'x': plain words")).toBe('plain words');
  });

  it('leaves a sentence without the wrapper as it is', () => {
    expect(ipcErrorMessage('Error: the daemon said no')).toBe('Error: the daemon said no');
    expect(ipcErrorMessage('  Paused  ')).toBe('Paused');
  });
});

describe('unwrapIpcError', () => {
  it('rethrows the main process sentence as a plain Error, keeping the original as its cause', () => {
    const original = new Error(WRAPPED);
    const unwrapped = unwrapIpcError(original) as Error & { cause?: unknown };
    expect(unwrapped).toBeInstanceOf(Error);
    expect(unwrapped.message).toBe('Finish the open Save or Open window first.');
    expect(unwrapped.cause).toBe(original);
  });

  it('gives the fallback for a wrapper with nothing inside', () => {
    expect(
      (
        unwrapIpcError(
          new Error("Error invoking remote method 'x': Error: "),
          'It failed.'
        ) as Error
      ).message
    ).toBe('It failed.');
  });

  it('returns anything else unchanged', () => {
    const plain = new Error('Crew refused it.');
    expect(unwrapIpcError(plain)).toBe(plain);
    expect(unwrapIpcError('text')).toBe('text');
    expect(unwrapIpcError(null)).toBeNull();
  });
});

describe('failureSentence', () => {
  it("is an Error's own sentence, unwrapped, else the fallback", () => {
    expect(failureSentence(new Error(WRAPPED), 'fallback')).toBe(
      'Finish the open Save or Open window first.'
    );
    expect(failureSentence(new Error(''), 'fallback')).toBe('fallback');
    expect(failureSentence({ message: 'not an Error' }, 'fallback')).toBe('fallback');
  });
});
