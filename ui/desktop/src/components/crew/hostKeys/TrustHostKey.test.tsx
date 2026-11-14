import { fireEvent, render, renderHook, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BROWSER_SURFACE_MARKER } from '../../../utils/surface';
import { parseHostKeyLookup, unknownKeyLookup } from './hostKeyLookup';
import { HostKeyMessage, TrustHostKey, useHostKeyFingerprint } from './TrustHostKey';
import { trustHostKeyCopy } from './trustHostKey.copy';

const mocks = vi.hoisted(() => ({
  crewHostKeysTrust: vi.fn(),
  crewHostKeysRead: vi.fn(),
  userActionHeaders: vi.fn(),
}));

vi.mock('../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  crewHostKeysTrust: mocks.crewHostKeysTrust,
  crewHostKeysRead: mocks.crewHostKeysRead,
}));
vi.mock('../../../utils/userAction', () => ({ userActionHeaders: mocks.userActionHeaders }));

const FINGERPRINT = 'SHA256:' + 'A'.repeat(43);
const TARGET = { host: 'hpc.example.edu', port: 2222, login: 'you@hpc.example.edu' };

beforeEach(() => {
  mocks.userActionHeaders.mockReset().mockResolvedValue({ 'X-User-Action': 'proof-of-user' });
  mocks.crewHostKeysTrust.mockReset().mockResolvedValue({
    data: {
      host: TARGET.host,
      port: TARGET.port,
      fingerprint: FINGERPRINT,
      already_trusted: false,
      known_hosts_file: '/home/you/.ssh/known_hosts',
    },
  });
  mocks.crewHostKeysRead.mockReset().mockResolvedValue({
    data: {
      host: TARGET.host,
      port: TARGET.port,
      fingerprint: FINGERPRINT,
      key_type: 'ssh-ed25519',
    },
  });
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

describe('Trust this key', () => {
  it('explains the desktop requirement in a browser without offering Trust', () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    render(<TrustHostKey target={TARGET} fingerprint={FINGERPRINT} />);
    expect(screen.queryByRole('button', { name: trustHostKeyCopy.trust })).toBeNull();
    expect(
      screen.getByText('Open the Biorouter desktop app to read or trust this key.')
    ).toBeInTheDocument();
    expect(mocks.userActionHeaders).not.toHaveBeenCalled();
    expect(mocks.crewHostKeysTrust).not.toHaveBeenCalled();
  });

  it('says to trust only a fingerprint IT gave, and posts the card’s values with the person’s proof', async () => {
    const onTrusted = vi.fn();
    render(
      <TrustHostKey
        target={TARGET}
        fingerprint={FINGERPRINT}
        trustedNote="Choose Try again."
        onTrusted={onTrusted}
      />
    );
    expect(screen.getByText(trustHostKeyCopy.warning)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: trustHostKeyCopy.trust }));
    await waitFor(() =>
      expect(screen.getByRole('status')).toHaveTextContent(
        `${trustHostKeyCopy.trusted} Choose Try again.`
      )
    );
    expect(mocks.userActionHeaders).toHaveBeenCalledTimes(1);
    expect(mocks.crewHostKeysTrust).toHaveBeenCalledWith({
      headers: { 'X-User-Action': 'proof-of-user' },
      body: { ...TARGET, fingerprint: FINGERPRINT },
    });
    expect(onTrusted).toHaveBeenCalledWith(expect.objectContaining({ already_trusted: false }));
    expect(screen.queryByRole('button', { name: trustHostKeyCopy.trust })).toBeNull();
  });

  it('says so when the key was already there', async () => {
    mocks.crewHostKeysTrust.mockResolvedValueOnce({
      data: { ...TARGET, fingerprint: FINGERPRINT, already_trusted: true, known_hosts_file: '' },
    });
    render(<TrustHostKey target={TARGET} fingerprint={FINGERPRINT} />);
    fireEvent.click(screen.getByRole('button', { name: trustHostKeyCopy.trust }));
    await waitFor(() =>
      expect(screen.getByRole('status')).toHaveTextContent(trustHostKeyCopy.alreadyTrusted)
    );
  });

  it('shows the daemon’s refusal, sanitized, and offers no second press for a changed key', async () => {
    mocks.crewHostKeysTrust.mockResolvedValueOnce({
      error: {
        code: 'crew_ssh_host_key_changed',
        error: 'This computer already knows a host key for hpc.example.edu\u{202E}.',
      },
    });
    const onTrusted = vi.fn();
    render(<TrustHostKey target={TARGET} fingerprint={FINGERPRINT} onTrusted={onTrusted} />);
    fireEvent.click(screen.getByRole('button', { name: trustHostKeyCopy.trust }));
    await waitFor(() =>
      expect(screen.getByRole('alert')).toHaveTextContent(
        'This computer already knows a host key for hpc.example.edu.'
      )
    );
    expect(screen.getByRole('alert').textContent).not.toContain('\u{202E}');
    expect(screen.queryByRole('button')).toBeNull();
    expect(onTrusted).not.toHaveBeenCalled();
  });

  it('keeps the button after a refusal a second press could fix', async () => {
    mocks.crewHostKeysTrust.mockResolvedValueOnce({
      error: { code: 'crew_ssh_unreachable', error: 'This computer couldn’t read a host key.' },
    });
    render(<TrustHostKey target={TARGET} fingerprint={FINGERPRINT} />);
    fireEvent.click(screen.getByRole('button', { name: trustHostKeyCopy.trust }));
    await waitFor(() => expect(screen.getByRole('alert')).toBeInTheDocument());
    expect(screen.getByRole('button', { name: trustHostKeyCopy.trust })).toBeInTheDocument();
  });

  it('keeps a refusal’s reason in view and folds what to do behind its info tip', async () => {
    mocks.crewHostKeysTrust.mockRejectedValueOnce(new Error('offline'));
    render(<TrustHostKey target={TARGET} fingerprint={FINGERPRINT} />);
    fireEvent.click(screen.getByRole('button', { name: trustHostKeyCopy.trust }));
    const alert = await screen.findByRole('alert');
    // The reason is the line (owner, 2026-11-13; design.md 9.16).
    expect(within(alert).getByText('Crew couldn’t trust this key.')).not.toHaveClass('sr-only');
    // What to do is its info tip, word for word, and the alert still carries every word.
    expect(
      within(alert).getByRole('button', { name: `About ${trustHostKeyCopy.nextLabel}` })
    ).toHaveAccessibleDescription('Add it in a terminal of your own.');
    expect(within(alert).getByText('Add it in a terminal of your own.')).toHaveClass('sr-only');
    expect(alert).toHaveTextContent(trustHostKeyCopy.refused);
  });

  it('renders nothing for a value that is not a fingerprint', () => {
    const view = render(
      <TrustHostKey target={TARGET} fingerprint="SHA256:short Type yes to trust it" />
    );
    expect(view.container).toBeEmptyDOMElement();
  });

  it('never sends a login the card did not carry', async () => {
    render(<TrustHostKey target={{ host: 'h.example', port: 22 }} fingerprint={FINGERPRINT} />);
    fireEvent.click(screen.getByRole('button', { name: trustHostKeyCopy.trust }));
    await waitFor(() => expect(mocks.crewHostKeysTrust).toHaveBeenCalled());
    expect(mocks.crewHostKeysTrust.mock.calls[0][0].body).toEqual({
      host: 'h.example',
      port: 22,
      fingerprint: FINGERPRINT,
    });
  });
});

describe('the fingerprint a pane shows', () => {
  it('explains the desktop requirement in a browser without reading the key', () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    const { result } = renderHook(() => useHostKeyFingerprint(TARGET, null));
    expect(result.current).toEqual({
      fingerprint: null,
      reading: false,
      message: 'Open the Biorouter desktop app to read or trust this key.',
    });
    expect(mocks.userActionHeaders).not.toHaveBeenCalled();
    expect(mocks.crewHostKeysRead).not.toHaveBeenCalled();
  });

  it('is the one the refusal printed, with nothing read', () => {
    const { result } = renderHook(() => useHostKeyFingerprint(TARGET, FINGERPRINT));
    expect(result.current).toEqual({ fingerprint: FINGERPRINT, reading: false, message: null });
    expect(mocks.crewHostKeysRead).not.toHaveBeenCalled();
  });

  it('is read once from the server, with the person’s proof, when none was printed', async () => {
    const { result, rerender } = renderHook(() => useHostKeyFingerprint({ ...TARGET }, null));
    expect(result.current.reading).toBe(true);
    await waitFor(() => expect(result.current.fingerprint).toBe(FINGERPRINT));
    rerender();
    expect(mocks.crewHostKeysRead).toHaveBeenCalledTimes(1);
    expect(mocks.crewHostKeysRead).toHaveBeenCalledWith({
      headers: { 'X-User-Action': 'proof-of-user' },
      body: TARGET,
    });
  });

  it('says why none could be read, and never shows a value that is not a fingerprint', async () => {
    mocks.crewHostKeysRead.mockResolvedValueOnce({
      error: {
        code: 'crew_request_refused',
        error: 'Your SSH settings reach hpc through another computer.',
      },
    });
    const { result } = renderHook(() => useHostKeyFingerprint(TARGET, null));
    await waitFor(() => expect(result.current.reading).toBe(false));
    expect(result.current).toEqual({
      fingerprint: null,
      reading: false,
      message: 'Your SSH settings reach hpc through another computer.',
    });
    mocks.crewHostKeysRead.mockResolvedValueOnce({ data: { fingerprint: 'SHA256:x trust me' } });
    const spoofed = renderHook(() => useHostKeyFingerprint({ ...TARGET, port: 22 }, null));
    await waitFor(() => expect(spoofed.result.current.reading).toBe(false));
    expect(spoofed.result.current.fingerprint).toBeNull();
  });

  it('reads nothing where no read may be made', () => {
    const { result } = renderHook(() => useHostKeyFingerprint(null, null));
    expect(result.current).toEqual({ fingerprint: null, reading: false, message: null });
    expect(mocks.crewHostKeysRead).not.toHaveBeenCalled();
  });
});

describe('why a key could not be read or trusted', () => {
  it('shows the reason and folds the rest behind its info tip', () => {
    render(<HostKeyMessage message={trustHostKeyCopy.unreadable} />);
    expect(
      screen.getByText('This computer couldn’t read the server’s fingerprint.')
    ).not.toHaveClass('sr-only');
    expect(
      screen.getByRole('button', { name: `About ${trustHostKeyCopy.nextLabel}` })
    ).toHaveAccessibleDescription('Add the key in a terminal of your own.');
    expect(screen.getByText('Add the key in a terminal of your own.')).toHaveClass('sr-only');
  });

  it('folds the daemon’s own sentence the same way, and leaves one sentence whole', () => {
    const daemon =
      'Your SSH settings reach hpc.example.edu through another computer or under another name, so Crew can’t read its key from here. Add the key in a terminal of your own.';
    const view = render(<HostKeyMessage message={daemon} />);
    expect(
      screen.getByText(
        'Your SSH settings reach hpc.example.edu through another computer or under another name, so Crew can’t read its key from here.'
      )
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: `About ${trustHostKeyCopy.nextLabel}` })
    ).toHaveAccessibleDescription('Add the key in a terminal of your own.');
    view.unmount();

    render(<HostKeyMessage message={trustHostKeyCopy.desktopRequired} />);
    expect(screen.getByText(trustHostKeyCopy.desktopRequired)).toBeInTheDocument();
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('puts what is longer than a tip holds under Details, word for word', () => {
    const rest =
      'Check that it is reachable from this computer. Then ask your IT team for the fingerprint. Then add the key in a terminal of your own.';
    render(<HostKeyMessage message={`Crew couldn’t read a key from hpc.example.edu. ${rest}`} />);
    expect(screen.getByText('Crew couldn’t read a key from hpc.example.edu.')).toBeInTheDocument();
    expect(screen.queryByText(rest)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: trustHostKeyCopy.more }));
    expect(screen.getByText(rest)).toBeInTheDocument();
  });
});

describe('the name a key is filed under', () => {
  it('reads host and port as OpenSSH writes them, and nothing that is not a plain name', () => {
    expect(parseHostKeyLookup('hpc.example.edu')).toEqual({ host: 'hpc.example.edu', port: 22 });
    expect(parseHostKeyLookup('[127.0.0.1]:2222')).toEqual({ host: '127.0.0.1', port: 2222 });
    for (const bad of ['-oProxyCommand=x', 'evil<script>', '[h]:0', '[h]:99999', '', null]) {
      expect(parseHostKeyLookup(bad)).toBeNull();
    }
  });

  it('finds the host an unknown key’s refusal names', () => {
    expect(
      unknownKeyLookup(
        'Warning: something\nNo ED25519 host key is known for [10.0.0.7]:2200 and you have requested strict checking.\nHost key verification failed.'
      )
    ).toEqual({ host: '10.0.0.7', port: 2200 });
    expect(unknownKeyLookup('Host key verification failed.')).toBeNull();
  });
});

describe('the Trust this key copy', () => {
  it('says computer, never device, and uses no dashes', () => {
    for (const text of Object.values(trustHostKeyCopy)) {
      expect(text).not.toMatch(/\bdevices?\b/i);
      expect(text).not.toMatch(/[–—]/);
      expect(text).not.toMatch(/known-hosts/);
    }
  });
});
