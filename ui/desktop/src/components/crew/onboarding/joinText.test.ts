import { describe, expect, it } from 'vitest';
import { joinerPerson, personFromProjection } from '../identity';
import {
  firstName,
  groupWorkspaceFingerprint,
  hostKeyFingerprints,
  hostStartCommands,
  isUnknownDeviceFailure,
  isWorkspaceName,
  sshLoginCommand,
  sshUsername,
  workspaceSlug,
} from './joinText';

describe('sshUsername', () => {
  it('reads the login of a user@host target and nothing from a bare alias', () => {
    expect(sshUsername('bob@hpc.ucsf.edu')).toBe('bob');
    expect(sshUsername('hpc-alias')).toBeNull();
    expect(sshUsername('@host')).toBeNull();
    expect(sshUsername(undefined)).toBeNull();
  });
});

describe('firstName', () => {
  it('uses the first word of a chosen name, else the @username', () => {
    expect(firstName(personFromProjection({ username: 'alice', display_name: 'Alice Chen' }))).toBe(
      'Alice'
    );
    expect(firstName(joinerPerson('alice'))).toBe('@alice');
    expect(firstName(null)).toBeNull();
  });
});

describe('groupWorkspaceFingerprint', () => {
  it('groups the first sixteen hex digits in fours, upper-cased', () => {
    expect(groupWorkspaceFingerprint('3f2a9c1e77b0d4e1' + 'f'.repeat(48))).toBe(
      '3F2A 9C1E 77B0 D4E1'
    );
    expect(groupWorkspaceFingerprint('3F2A 9C1E 77B0 D4E1')).toBe('3F2A 9C1E 77B0 D4E1');
    expect(groupWorkspaceFingerprint('abc')).toBeNull();
  });
});

describe('hostKeyFingerprints', () => {
  const offered = 'SHA256:' + 'A'.repeat(43);
  const known = 'SHA256:' + 'B'.repeat(43);

  it('reads the fingerprints the daemon summarized for a changed key', () => {
    const detail = [
      `New host key fingerprint (offered by the server): ${offered}`,
      `Previously known host key fingerprint: ${known}`,
      '',
      '@@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@@',
    ].join('\n');
    expect(hostKeyFingerprints(detail)).toEqual({ offered, known });
  });

  it('reads an offered or unlabeled fingerprint for an unknown key', () => {
    expect(hostKeyFingerprints(`Offered host key fingerprint: ${offered}`)).toEqual({
      offered,
      known: null,
    });
    expect(hostKeyFingerprints(`Host key fingerprint: ${offered}`)).toEqual({
      offered,
      known: null,
    });
  });

  it('finds nothing in text that only mentions a fingerprint', () => {
    expect(hostKeyFingerprints(`The key ${offered} was rejected`)).toEqual({
      offered: null,
      known: null,
    });
    expect(hostKeyFingerprints(undefined)).toEqual({ offered: null, known: null });
  });
});

describe('workspace names', () => {
  it('previews the slug a typed name becomes', () => {
    expect(workspaceSlug('Lab Data')).toBe('lab-data');
    expect(workspaceSlug('  --Café lab!! ')).toBe('cafe-lab');
    expect(workspaceSlug('x'.repeat(50))).toHaveLength(40);
    expect(workspaceSlug('!!!')).toBe('');
  });

  it('accepts only the S2 workspace-name rule', () => {
    expect(isWorkspaceName('lab')).toBe(true);
    expect(isWorkspaceName('lab-2')).toBe(true);
    expect(isWorkspaceName('-lab')).toBe(false);
    expect(isWorkspaceName('Lab')).toBe(false);
    expect(isWorkspaceName('11111111-2222-3333-4444-555555555555')).toBe(false);
  });
});

describe('hostStartCommands', () => {
  it('starts Crew with the name and the hosting key, and prints the status an older broker needs', () => {
    const commands = hostStartCommands('lab', 'k'.repeat(64));
    expect(commands).toContain(
      `"$HOME/.local/bin/biorouter-crew" start --state-dir "$HOME/.local/share/biorouter-crew/lab" --name lab --bootstrap-key ${'k'.repeat(64)}`
    );
    expect(commands).toContain(
      '"$HOME/.local/bin/biorouter-crew" status --state-dir "$HOME/.local/share/biorouter-crew/lab"'
    );
    expect(commands.split('\n')[0]).toBe('umask 077');
  });
});

describe('sshLoginCommand', () => {
  it('signs in the way the connection will', () => {
    expect(sshLoginCommand({ ssh_target: 'alice@hpc.ucsf.edu' })).toBe('ssh alice@hpc.ucsf.edu');
    expect(
      sshLoginCommand({
        ssh_target: 'alice@hpc.ucsf.edu',
        port: 2222,
        proxy_jump: 'gw.ucsf.edu',
        identity_file: '~/.ssh/id ed25519',
      })
    ).toBe("ssh -p 2222 -J gw.ucsf.edu -i '~/.ssh/id ed25519' alice@hpc.ucsf.edu");
  });
});

describe('isUnknownDeviceFailure', () => {
  it('recognizes only the broker refusing an unknown device', () => {
    expect(isUnknownDeviceFailure('unauthorized: unknown device Your unsent draft…')).toBe(true);
    expect(isUnknownDeviceFailure('unauthorized: account enrollment changed')).toBe(false);
    expect(isUnknownDeviceFailure(null)).toBe(false);
  });
});
