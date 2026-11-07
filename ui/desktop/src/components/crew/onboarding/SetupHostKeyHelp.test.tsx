import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { trustHostKeyCopy } from '../hostKeys/trustHostKey.copy';
import { trustCopy } from './copy';
import { SetupHostKeyHelp } from './SetupWorkspaces';

const keys = vi.hoisted(() => ({ read: vi.fn(), trust: vi.fn() }));
vi.mock('../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  crewHostKeysRead: keys.read,
  crewHostKeysTrust: keys.trust,
}));

const OFFERED = 'SHA256:' + 'A'.repeat(43);
const KNOWN = 'SHA256:' + 'B'.repeat(43);
/** The job's own login, which the daemon reads the server's SSH settings for. */
const ROUTE = { login: 'you@leo.ucsf.edu', proxyJump: null };
const UNKNOWN_DETAIL =
  'No ED25519 host key is known for [leo.ucsf.edu]:2222 and you have requested strict checking.\nHost key verification failed.';

beforeEach(() => {
  keys.read.mockReset().mockResolvedValue({
    data: { host: 'leo.ucsf.edu', port: 2222, fingerprint: OFFERED, key_type: 'ssh-ed25519' },
  });
  keys.trust.mockReset().mockResolvedValue({
    data: {
      host: 'leo.ucsf.edu',
      port: 2222,
      fingerprint: OFFERED,
      already_trusted: false,
      known_hosts_file: '/home/you/.ssh/known_hosts',
    },
  });
});

describe('a setup that stopped on a host key (D2)', () => {
  it('reads the fingerprint from the server OpenSSH named, labelled as this computer’s, and trusts it', async () => {
    render(
      <SetupHostKeyHelp
        code="crew_ssh_host_key_unknown"
        detail={UNKNOWN_DETAIL}
        server="leo"
        route={ROUTE}
      />
    );
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: `Copy ${trustCopy.offeredLabel}` })
      ).toBeInTheDocument()
    );
    expect(keys.read).toHaveBeenCalledWith({
      headers: expect.any(Object),
      body: { host: 'leo.ucsf.edu', port: 2222, login: 'you@leo.ucsf.edu' },
    });
    expect(screen.getByText('Fingerprint this computer read from the server')).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/the server offered/i);
    fireEvent.click(screen.getByRole('button', { name: trustHostKeyCopy.trust }));
    await waitFor(() =>
      expect(screen.getByRole('status')).toHaveTextContent(
        `${trustHostKeyCopy.trusted} ${trustHostKeyCopy.tryAgainNote}`
      )
    );
    expect(keys.trust).toHaveBeenCalledWith({
      headers: expect.any(Object),
      body: { host: 'leo.ucsf.edu', port: 2222, login: 'you@leo.ucsf.edu', fingerprint: OFFERED },
    });
  });

  it('reads nothing when the refusal names no host, and offers nothing to trust', () => {
    render(
      <SetupHostKeyHelp
        code="crew_ssh_host_key_unknown"
        detail="Host key verification failed."
        server="leo"
        route={ROUTE}
      />
    );
    expect(keys.read).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: trustHostKeyCopy.trust })).toBeNull();
  });

  it('never reads or trusts a key behind the job’s jump host, or where its route is unknown', () => {
    for (const route of [{ login: 'you@leo.ucsf.edu', proxyJump: 'gate.ucsf.edu' }, null]) {
      const { unmount } = render(
        <SetupHostKeyHelp
          code="crew_ssh_host_key_unknown"
          detail={`Offered host key fingerprint: ${OFFERED}\n${UNKNOWN_DETAIL}`}
          server="leo"
          route={route}
        />
      );
      expect(
        screen.getByRole('button', { name: `Copy ${trustCopy.offeredLabel}` })
      ).toBeInTheDocument();
      expect(keys.read).not.toHaveBeenCalled();
      expect(screen.queryByRole('button', { name: trustHostKeyCopy.trust })).toBeNull();
      unmount();
    }
  });

  it('never reads or trusts a changed key', () => {
    render(
      <SetupHostKeyHelp
        code="crew_ssh_host_key_changed"
        detail={`New host key fingerprint (offered by the server): ${OFFERED}\nPreviously known host key fingerprint: ${KNOWN}`}
        server="leo"
        route={ROUTE}
      />
    );
    expect(screen.getByTestId('crew-setup-host-key-changed')).toBeInTheDocument();
    expect(screen.getByText(trustCopy.changedBody)).toBeInTheDocument();
    expect(trustCopy.changedBody).toContain('known hosts file');
    expect(keys.read).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: trustHostKeyCopy.trust })).toBeNull();
    expect(screen.queryByText(trustHostKeyCopy.warning)).toBeNull();
  });
});
