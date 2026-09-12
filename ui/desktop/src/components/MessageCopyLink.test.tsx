import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';

const mocks = vi.hoisted(() => ({ toastError: vi.fn() }));
vi.mock('../toasts', () => ({ toastError: mocks.toastError }));

import MessageCopyLink from './MessageCopyLink';

/** No rich node, so the component takes the plain-text path. */
const noContent = { current: null };

/**
 * ⚠ **Call this AFTER `userEvent.setup()`.** `setup()` installs a clipboard stub
 * of its own, so a stub written first is replaced by one whose `writeText`
 * resolves — and every failure test quietly measures a success.
 */
function stubClipboard(writeText: () => Promise<void>) {
  // `Object.assign` works once and then trips over the prototype getter, which
  // makes the second test in the file fail for a reason that has nothing to do
  // with what it is testing.
  Object.defineProperty(navigator, 'clipboard', {
    value: { writeText: vi.fn(writeText), write: vi.fn() },
    configurable: true,
    writable: true,
  });
}

beforeEach(() => {
  mocks.toastError.mockClear();
  // The app's own logging stays quiet; these paths log on purpose.
  vi.spyOn(console, 'error').mockImplementation(() => {});
});

afterEach(() => vi.restoreAllMocks());

/**
 * The reported defect: when both clipboard writes fail, the outer catch logged
 * `'Failed to copy text: '`, the inner catch logged `'Failed to copy text
 * (fallback): '`, and that was the end of it. `markCopied()` was never reached,
 * so the button went on saying "Copy" — the user's only way to learn nothing had
 * been copied was to paste somewhere and find out.
 */
describe('MessageCopyLink', () => {
  it('says so when the clipboard refuses', async () => {
    const user = userEvent.setup();
    stubClipboard(() => Promise.reject(new Error('denied')));
    render(<MessageCopyLink text="hello" contentRef={noContent} />);

    await user.click(screen.getByRole('button', { name: 'Copy message' }));

    await waitFor(() => expect(screen.getByRole('button')).toHaveTextContent('Copy failed'));
    expect(mocks.toastError).toHaveBeenCalledTimes(1);
    expect(mocks.toastError.mock.calls[0][0]).toMatchObject({ title: 'Copy failed' });
  });

  it('leaves the success path alone', async () => {
    const user = userEvent.setup();
    stubClipboard(() => Promise.resolve());
    render(<MessageCopyLink text="hello" contentRef={noContent} />);

    await user.click(screen.getByRole('button', { name: 'Copy message' }));

    await waitFor(() => expect(screen.getByRole('button')).toHaveTextContent('Copied!'));
    expect(mocks.toastError).not.toHaveBeenCalled();
  });

  it('still counts a successful fallback as a copy', async () => {
    // The rich write is what fails here — jsdom has no `ClipboardItem`, which is
    // the same shape of failure as a browser refusing the `text/html` flavour —
    // and the plain-text retry succeeds.
    const user = userEvent.setup();
    stubClipboard(() => Promise.resolve());
    const contentRef = { current: document.createElement('div') };
    contentRef.current.textContent = 'hello';
    render(<MessageCopyLink text="hello" contentRef={contentRef} />);

    await user.click(screen.getByRole('button', { name: 'Copy message' }));

    await waitFor(() => expect(screen.getByRole('button')).toHaveTextContent('Copied!'));
    expect(mocks.toastError).not.toHaveBeenCalled();
  });
});
