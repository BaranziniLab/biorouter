import { describe, expect, it, vi } from 'vitest';
import { writeConversationId, writeSelectedText } from './conversationClipboard';

describe('native app clipboard writes', () => {
  it('writes the exact ID only for a registered app main frame', () => {
    const write = vi.fn();
    writeConversationId({ isAppWindow: true, isMainFrame: true }, '20260921_2', write);
    expect(write).toHaveBeenCalledWith('20260921_2');
    for (const sender of [
      { isAppWindow: false, isMainFrame: true },
      { isAppWindow: true, isMainFrame: false },
    ]) {
      expect(() => writeConversationId(sender, 'secret', write)).toThrow('main frame');
      expect(() => writeSelectedText(sender, 'secret', write)).toThrow('main frame');
    }
    expect(write).toHaveBeenCalledTimes(1);
  });
  it('rejects malformed and oversized input without a clipboard write', () => {
    const write = vi.fn();
    const sender = { isAppWindow: true, isMainFrame: true };
    for (const id of [null, {}, 1, '', 'x'.repeat(513)])
      expect(() => writeConversationId(sender, id, write)).toThrow();
    expect(write).not.toHaveBeenCalled();
    writeSelectedText(sender, 'quotation\n"exact"', write);
    expect(write).toHaveBeenCalledWith('quotation\n"exact"');
  });
});
