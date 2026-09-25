import { describe, expect, it } from 'vitest';
import { joinerPerson, personFromProjection } from '../identity';
import {
  connectionServerLabel,
  CREW_MEMBERSHIP_ENDED,
  firstName,
  groupWorkspaceFingerprint,
  hostKeyFingerprints,
  hostStartCommands,
  isUnknownDeviceFailure,
  isWorkspaceName,
  membershipEnded,
  readStartOutput,
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

  it('carries no shell syntax from whatever the host typed as the name (D-HOST)', () => {
    // The daemon runs its own copy of this text, built from the same two values under the same
    // grammar, and refuses anything else; the dialog shows only what the slug rule lets through.
    for (const typed of [
      'Lab Data',
      'lab; rm -rf ~',
      '$(id)',
      '`id`',
      'a"b',
      "a'b",
      'lab\nwhoami',
      'ÜBER lab',
      '../../etc',
    ]) {
      const slug = workspaceSlug(typed);
      expect(slug).toMatch(/^[a-z0-9-]*$/);
      const commands = hostStartCommands(slug, 'c'.repeat(64));
      expect(commands.split('\n')).toHaveLength(4);
      expect(commands).not.toMatch(/[;`|&<>]|\$\(/);
    }
  });
});

describe('connectionServerLabel', () => {
  it('prefers the daemon’s name for the server, the person’s own SSH alias (D-ALIAS)', () => {
    expect(
      connectionServerLabel({ ssh_target: 'bob@52.33.141.141', server_label: 'lab-server' })
    ).toBe('lab-server');
  });

  it('falls back to the login’s host when the daemon named none', () => {
    expect(connectionServerLabel({ ssh_target: 'bob@hpc.ucsf.edu' })).toBe('hpc.ucsf.edu');
    expect(connectionServerLabel({ ssh_target: 'bob@hpc.ucsf.edu', server_label: '' })).toBe(
      'hpc.ucsf.edu'
    );
    expect(connectionServerLabel({ ssh_target: 'bob@hpc.ucsf.edu', server_label: 7 })).toBe(
      'hpc.ucsf.edu'
    );
    expect(connectionServerLabel(null)).toBe('');
  });

  it('shows a label only as display text', () => {
    expect(
      connectionServerLabel({ ssh_target: 'bob@hpc', server_label: 'lab\u202eserver\u0007' })
    ).toBe('labserver');
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

describe('readStartOutput', () => {
  const STATUS = '{"workspace_id":"w-1","socket":"/tmp/crew-1000-abc/broker.sock","host_uid":1000}';
  const TOKEN = 'brcrew1:eyJ2IjoxLCJ3b3Jrc3BhY2VfaWQiOiIuLi4ifQ';

  it('takes the invitation line out of start’s JSON, rejoined when the copy broke it', () => {
    const start = `{"started_pid":4242,"state":"running","invitation":"${TOKEN}","name":"lab"}`;
    expect(readStartOutput(`alice@hpc:~$ ${start}\nalice@hpc:~$ `)).toEqual({
      kind: 'text',
      text: TOKEN,
    });
    const broken = start.replace('eyJ2IjoxLCJ3b3Jr', 'eyJ2IjoxLC\nJ3b3Jr');
    expect(readStartOutput(broken)).toEqual({ kind: 'text', text: TOKEN });
  });

  it('leaves a bare invitation line for the daemon to find', () => {
    const message = `Join lab on Crew.\n${TOKEN}`;
    expect(readStartOutput(message)).toEqual({ kind: 'text', text: message });
  });

  it('finds the status JSON inside prompts, other output and a stray brace, on one line', () => {
    const pasted = [
      'alice@hpc ~ ${PWD',
      'started pid 4242',
      '{"workspace_id":"w-1","socket":"/tmp/crew-1000-abc/bro',
      'ker.sock","host_uid":1000}',
      'alice@hpc:~$ ',
    ].join('\n');
    expect(readStartOutput(pasted)).toEqual({ kind: 'text', text: STATUS });
    expect(readStartOutput(`  ${STATUS}  `)).toEqual({ kind: 'text', text: STATUS });
  });

  it('names what it can see is wrong', () => {
    expect(readStartOutput('{"workspace_id":"w-1","socket":"/tmp/crew-')).toEqual({
      kind: 'problem',
      problem: 'cut-off',
    });
    expect(
      readStartOutput('{"started_pid":4242,"state":"starting","status_command":"status"}')
    ).toEqual({ kind: 'problem', problem: 'starting' });
    for (const shell of [
      'bash: /home/alice/.local/bin/biorouter-crew: No such file or directory',
      'zsh: no such file or directory: /home/alice/.local/bin/biorouter-crew',
      'sh: 1: biorouter-crew: not found',
    ]) {
      expect(readStartOutput(shell)).toEqual({ kind: 'problem', problem: 'not-installed' });
    }
    expect(
      readStartOutput('Error: name_mismatch: this workspace already has another name')
    ).toEqual({
      kind: 'problem',
      problem: 'server-error',
      detail: 'name_mismatch: this workspace already has another name',
    });
    expect(
      readStartOutput(
        '{"started_pid":1,"state":"running","invitation":null,"invitation_error":"no hello"}'
      )
    ).toEqual({ kind: 'problem', problem: 'server-error', detail: 'no hello' });
  });

  it('hands anything else to the daemon as it is, and nothing for an empty paste', () => {
    expect(readStartOutput('oops')).toEqual({ kind: 'text', text: 'oops' });
    expect(readStartOutput('{"unrelated": true}')).toEqual({
      kind: 'text',
      text: '{"unrelated": true}',
    });
    expect(readStartOutput('   \n ')).toBeNull();
  });
});

describe('membershipEnded (Q3-50)', () => {
  it('reads the daemon’s record that the workspace ended this membership, and nothing else', () => {
    expect(CREW_MEMBERSHIP_ENDED).toBe('crew_membership_ended');
    expect(membershipEnded({ id: 'c', last_error_code: 'crew_membership_ended' })).toBe(true);
    // Another failure, a daemon that predates the field, and no connection at all say nothing.
    expect(membershipEnded({ id: 'c', last_error_code: 'crew_ssh_auth_required' })).toBe(false);
    expect(membershipEnded({ id: 'c' })).toBe(false);
    expect(membershipEnded({ id: 'c', last_error_code: null })).toBe(false);
    expect(membershipEnded(null)).toBe(false);
    expect(membershipEnded(undefined)).toBe(false);
  });
});
