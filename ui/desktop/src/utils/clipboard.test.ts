import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CLIPBOARD_RETRY_DELAY_MS, copyToClipboard } from './clipboard';

/**
 * The shared copy path every in-place Copy control uses (a fenced block's Copy, the artifact
 * panel's Copy, CopyField, a message's Copy). What these pin is the ORDER: a refused write is
 * retried once after focusing the window, and only then does the document's own copy get a turn;
 * `false` means all three refused, and it is the only thing that lets a control say "Copy failed".
 */
describe('copyToClipboard', () => {
  let writeText: ReturnType<typeof vi.fn>;
  let execCommand: ReturnType<typeof vi.fn>;
  let host: HTMLDivElement;

  beforeEach(() => {
    writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      writable: true,
      value: { writeText },
    });
    execCommand = vi.fn(() => true);
    Object.defineProperty(document, 'execCommand', { value: execCommand, configurable: true });
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    host.remove();
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('writes once when the clipboard accepts', async () => {
    await expect(copyToClipboard('abc', host)).resolves.toBe(true);
    expect(writeText).toHaveBeenCalledExactlyOnceWith('abc');
    expect(execCommand).not.toHaveBeenCalled();
  });

  it('focuses the window and retries once, after the delay, before any fallback', async () => {
    vi.useFakeTimers();
    const focus = vi.spyOn(window, 'focus').mockImplementation(() => {});
    writeText.mockRejectedValueOnce(new Error('denied'));

    const pending = copyToClipboard('abc', host);
    await vi.advanceTimersByTimeAsync(CLIPBOARD_RETRY_DELAY_MS - 1);
    expect(writeText).toHaveBeenCalledTimes(1);
    expect(focus).toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);

    await expect(pending).resolves.toBe(true);
    expect(writeText).toHaveBeenCalledTimes(2);
    expect(execCommand).not.toHaveBeenCalled();
  });

  it('falls back to the document copy of a hidden selection inside the host', async () => {
    writeText.mockRejectedValue(new Error('Write permission denied'));
    const button = document.createElement('button');
    host.appendChild(button);
    button.focus();
    let seen: { text: string; selected: string; inHost: boolean } | null = null;
    execCommand.mockImplementation((command: string) => {
      const area = document.activeElement as HTMLTextAreaElement;
      seen = {
        text: area.value,
        selected: area.value.slice(area.selectionStart, area.selectionEnd),
        inHost: host.contains(area),
      };
      return command === 'copy';
    });

    await expect(copyToClipboard('print("hi")', host)).resolves.toBe(true);

    expect(writeText).toHaveBeenCalledTimes(2);
    expect(execCommand).toHaveBeenCalledWith('copy');
    expect(seen).toEqual({ text: 'print("hi")', selected: 'print("hi")', inHost: true });
    // Nothing left behind, and focus is back on the control that was pressed.
    expect(host.querySelector('textarea')).toBeNull();
    expect(button).toHaveFocus();
  });

  it('is false only when the retry and the fallback have both refused', async () => {
    writeText.mockRejectedValue(new Error('denied'));
    execCommand.mockReturnValue(false);
    await expect(copyToClipboard('abc', host)).resolves.toBe(false);
    expect(writeText).toHaveBeenCalledTimes(2);
    expect(execCommand).toHaveBeenCalledTimes(1);
  });

  it('is false, without a fallback, when there is no host to put the selection in', async () => {
    writeText.mockRejectedValue(new Error('denied'));
    await expect(copyToClipboard('abc', null)).resolves.toBe(false);
    expect(execCommand).not.toHaveBeenCalled();
  });

  it('treats a missing clipboard API as a refusal, not a crash', async () => {
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      writable: true,
      value: undefined,
    });
    await expect(copyToClipboard('abc', host)).resolves.toBe(true);
    expect(execCommand).toHaveBeenCalledWith('copy');
  });
});
