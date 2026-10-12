import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ExternalBackendSection from './ExternalBackendSection';

/**
 * An external backend needs three secrets, and the component knew about two.
 *
 * `main.ts` reads `settings.externalBiorouterd.userActionKey` for
 * `getUserActionKey`, and the daemon compares SHA-256 of whatever the renderer
 * sends against the digest it was handed on stdin at launch. With no key here
 * the renderer sends nothing, and every private chat on that backend is refused
 * with a message telling the user to use the desktop app, which is the app they
 * are already in. Diagnostics, opening a private chat, and branching one all
 * fail the same way.
 *
 * The second test is the one that mattered most before the field existed. This
 * component's own shape carried only three fields and `saveConfig` writes the
 * whole object, so a key set by hand in `settings.json` was destroyed the next
 * time anyone touched the URL, the secret or the switch. That is silent data
 * loss on a credential, and it made the documented workaround unreliable too.
 */

const getSettings = vi.fn();
const saveSettings = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  saveSettings.mockResolvedValue(true);
  (window as unknown as { electron: unknown }).electron = { getSettings, saveSettings };
});

describe('ExternalBackendSection', () => {
  it('offers a user action key, without which private chats are unreachable', async () => {
    getSettings.mockResolvedValue({
      externalBiorouterd: { enabled: true, url: 'http://localhost:3000', secret: 's' },
    });

    render(<ExternalBackendSection />);
    const field = await screen.findByLabelText('User Action Key');
    expect(field).toBeVisible();

    fireEvent.change(field, { target: { value: 'proof-of-user' } });
    fireEvent.blur(field);

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalledWith(
        expect.objectContaining({
          externalBiorouterd: expect.objectContaining({ userActionKey: 'proof-of-user' }),
        })
      );
    });
  });

  it('draws the restart note’s lead word at 600, not preflight’s 700 (V4-R7-6)', async () => {
    getSettings.mockResolvedValue({
      externalBiorouterd: { enabled: true, url: 'http://localhost:3000', secret: 's' },
    });

    render(<ExternalBackendSection />);
    await screen.findByLabelText('User Action Key');
    // jsdom runs no cascade, so this pins the class; the weight itself is measured for every other
    // settings surface in Chromium and a bare `<strong>` there reads 700.
    expect(screen.getByText('Note:')).toHaveClass('font-semibold');
  });

  it('carries an existing key through a save of some other field', async () => {
    getSettings.mockResolvedValue({
      externalBiorouterd: {
        enabled: true,
        url: 'http://localhost:3000',
        secret: 's',
        userActionKey: 'set-by-hand',
      },
    });

    render(<ExternalBackendSection />);
    const secret = await screen.findByLabelText('Secret Key');
    fireEvent.change(secret, { target: { value: 'a-new-secret' } });
    fireEvent.blur(secret);

    await waitFor(() => expect(saveSettings).toHaveBeenCalled());
    // Index rather than `.at(-1)`: this project's tsc lib target does not
    // declare `Array.prototype.at`, and `npm run typecheck` is what CI runs.
    const calls = saveSettings.mock.calls;
    const written = calls[calls.length - 1][0] as {
      externalBiorouterd: { userActionKey?: string; secret?: string };
    };
    expect(written.externalBiorouterd.secret).toBe('a-new-secret');
    expect(written.externalBiorouterd.userActionKey).toBe('set-by-hand');
  });
});
