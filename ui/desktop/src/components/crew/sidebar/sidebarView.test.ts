import { isValidElement, type ReactElement, type ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type { PendingJoin, Snapshot } from '../crewApi';
import { buildPeopleDirectory } from '../identity';
import { sidebarCopy } from './copy';
import {
  expiryText,
  expiryWhen,
  forgetJoinerNames,
  joinedAsNamed,
  joinedLabel,
  joinerServerName,
  keepNamesWhole,
  knownUsername,
  loginLabel,
  rememberJoinerNames,
  serverLabel,
  waitingToJoin,
} from './sidebarView';

describe('waitingToJoin', () => {
  const snapshot = (pending_joins: unknown): Pick<Snapshot, 'pending_joins'> => ({
    pending_joins: pending_joins as PendingJoin[],
  });

  it('marks a join whose invitation ran out as expired, from the broker’s flag alone', () => {
    const rows = waitingToJoin(
      snapshot([
        { username: 'bob', full_name: 'Bob Lee', approved: true, expired: true },
        // The local clock is not asked: an `expires_at` in the past is the broker's to judge.
        { username: 'erin', expires_at: 1 },
        { username: 'finn', expired: 'yes' },
      ])
    );
    expect(rows.map((row) => [row.username, row.expired])).toEqual([
      ['bob', true],
      ['erin', false],
      ['finn', false],
    ]);
    expect(rows[0]).toMatchObject({ approved: true, serverName: 'Bob Lee' });
  });

  it('keeps only joins it can name, and nothing without the host’s list', () => {
    expect(waitingToJoin(null)).toEqual([]);
    expect(waitingToJoin(snapshot(undefined))).toEqual([]);
    expect(
      waitingToJoin(snapshot([null, { full_name: 'No username' }, { username: 'dee' }]))
    ).toEqual([
      {
        username: 'dee',
        serverName: null,
        expiresAt: null,
        approved: false,
        expired: false,
        otherDeviceTried: false,
      },
    ]);
  });
});

describe('an invitation’s expiry on the Waiting to join row (Q4-36)', () => {
  // Unix SECONDS, as the broker stamps it: `created_at + PENDING_JOIN_LIFETIME_SECS`.
  const NOW_MS = Date.UTC(2026, 8, 25, 1, 41);
  const IN_A_DAY = NOW_MS / 1000 + 86_400;

  it('reads expires_at as seconds, never as milliseconds', () => {
    const rows = waitingToJoin({
      pending_joins: [
        { username: 'jack', expires_at: IN_A_DAY },
        { username: 'kim', expires_at: 'soon' as unknown as number },
        { username: 'lee', expires_at: Number.NaN },
      ],
    });
    expect(rows.map((row) => row.expiresAt)).toEqual([IN_A_DAY, null, null]);
    // Read as milliseconds, a day from now would be January 1970 — long "expired".
    expect(expiryText(IN_A_DAY, NOW_MS)).toBe(sidebarCopy.waiting.expires(expiryWhen(IN_A_DAY)));
  });

  it('says "expires {weekday time}" in the contract’s format, then "expired" once past', () => {
    expect(expiryWhen(IN_A_DAY)).toBe(
      new Intl.DateTimeFormat(undefined, {
        weekday: 'short',
        hour: 'numeric',
        minute: '2-digit',
      }).format(new Date(IN_A_DAY * 1000))
    );
    expect(expiryText(IN_A_DAY, NOW_MS)).toMatch(/^expires \S/);
    expect(expiryText(NOW_MS / 1000, NOW_MS)).toBe('expired');
    expect(expiryText(NOW_MS / 1000 - 60, NOW_MS)).toBe('expired');
    expect(expiryText(null, NOW_MS)).toBeNull();
  });
});

describe('keepNamesWhole (Q4-51)', () => {
  /** The spans a result holds, as `[text, class]`, and the whole text it reads as. */
  function parts(node: ReactNode): { spans: [string, string][]; text: string } {
    const list = Array.isArray(node) ? node : [node];
    const spans: [string, string][] = [];
    let text = '';
    for (const part of list) {
      if (typeof part === 'string') text += part;
      else if (isValidElement(part)) {
        const element = part as ReactElement<{ className: string; children: string }>;
        spans.push([element.props.children, element.props.className]);
        text += element.props.children;
      }
    }
    return { spans, text };
  }

  it('holds each whole occurrence of a name, with the punctuation after it, and adds nothing', () => {
    const sentence =
      'Only the host can change chen-lab. Only people Alice lets in can see chen-lab…';
    const result = parts(keepNamesWhole(sentence, ['chen-lab']));
    expect(result.text).toBe(sentence);
    expect(result.spans).toEqual([
      ['chen-lab.', 'crew-sidebar-name'],
      ['chen-lab…', 'crew-sidebar-name'],
    ]);
  });

  it('leaves a name inside a longer word or slug alone', () => {
    expect(keepNamesWhole('The label says lab-2 and lab_x.', ['lab'])).toBe(
      'The label says lab-2 and lab_x.'
    );
    const result = parts(keepNamesWhole('lab, not label', ['lab']));
    expect(result.spans).toEqual([['lab,', 'crew-sidebar-name']]);
  });

  it('returns the text itself when there is no name to keep', () => {
    expect(keepNamesWhole('Private because your connection is Private.', ['chen-lab'])).toBe(
      'Private because your connection is Private.'
    );
    expect(keepNamesWhole('Anything', ['', null, undefined])).toBe('Anything');
    expect(keepNamesWhole('', ['chen-lab'])).toBe('');
  });

  it('prefers the longer of two names that start alike', () => {
    const result = parts(keepNamesWhole('see wong-lab-core today', ['wong-lab', 'wong-lab-core']));
    expect(result.spans).toEqual([['wong-lab-core', 'crew-sidebar-name']]);
  });
});

describe('the name on a joiner’s server account, kept past the join (Q4-42)', () => {
  afterEach(() => forgetJoinerNames());

  const workspace = {
    id: 'workspace-1',
    host_uid: 1000,
    mode: 'private' as const,
    policy_epoch: 1,
  };
  const jack = { id: 'person-jack', uid: 1004, username: 'crew_jack' };
  const host = { id: 'person-alice', uid: 1000, username: 'alice', nickname: 'Alice Chen' };
  const joinedDir = buildPeopleDirectory({ workspace, actor: host, principals: [host, jack] });

  it('remembers every waiting row a view lists, by workspace and username', () => {
    rememberJoinerNames({
      workspace,
      pending_joins: [
        { username: 'crew_jack', full_name: 'Jack Moreno' },
        { username: 'crew_kim' },
        { username: 'crew_lee', full_name: '   ' },
      ],
    });
    expect(joinerServerName('workspace-1', 'crew_jack')).toBe('Jack Moreno');
    expect(joinerServerName('workspace-1', 'crew_kim')).toBeNull();
    expect(joinerServerName('workspace-1', 'crew_lee')).toBeNull();
    // The same username in another workspace is another person.
    expect(joinerServerName('workspace-2', 'crew_jack')).toBeNull();
    // Nothing to remember without the workspace or the host's list.
    rememberJoinerNames({ workspace: { ...workspace, id: '' }, pending_joins: [] });
    rememberJoinerNames(null);
  });

  it('names someone who chose no name by it, once, and nobody else', () => {
    expect(joinedLabel(jack.id, joinedDir, 'workspace-1')).toBe('@crew_jack');
    expect(joinedAsNamed(jack.id, joinedDir, 'workspace-1')).toBeNull();
    rememberJoinerNames({
      workspace,
      pending_joins: [{ username: 'crew_jack', full_name: 'Jack Moreno' }],
    });
    expect(joinedLabel(jack.id, joinedDir, 'workspace-1')).toBe('Jack Moreno (@crew_jack)');
    expect(joinedLabel(jack.id, joinedDir, 'workspace-2')).toBe('@crew_jack');
    // A name the person chose wins: the server account's never replaces it (naming D2).
    const named = buildPeopleDirectory({
      workspace,
      actor: host,
      principals: [host, { ...jack, nickname: 'Jackie' }],
    });
    expect(joinedAsNamed(jack.id, named, 'workspace-1')).toBeNull();
    expect(joinedLabel(jack.id, named, 'workspace-1')).toBe('Jackie (@crew_jack)');
  });
});

describe('the server by the person’s own name for it (D-ALIAS)', () => {
  it('prefers the daemon’s label, and keeps the raw address out of both', () => {
    const saved = { ssh_target: 'crew_alice@52.33.141.141', server_label: 'lab-server' };
    expect(serverLabel(saved)).toBe('lab-server');
    expect(loginLabel(saved)).toBe('crew_alice@lab-server');
  });

  it('falls back to the saved login, exactly as saved, from a daemon that sends no label', () => {
    const saved = { ssh_target: 'alice@hpc.ucsf.edu' };
    expect(serverLabel(saved)).toBe('hpc.ucsf.edu');
    expect(loginLabel(saved)).toBe('alice@hpc.ucsf.edu');
    expect(serverLabel({ ssh_target: 'alice@hpc.ucsf.edu', server_label: '' })).toBe(
      'hpc.ucsf.edu'
    );
    expect(serverLabel({ ssh_target: 'alice@hpc.ucsf.edu', server_label: 42 })).toBe(
      'hpc.ucsf.edu'
    );
  });

  it('names an alias login by the alias alone', () => {
    const saved = { ssh_target: 'lab-server', server_label: 'lab-server' };
    expect(serverLabel(saved)).toBe('lab-server');
    expect(loginLabel(saved)).toBe('lab-server');
  });

  it('says nothing for no connection', () => {
    expect(serverLabel(null)).toBe('');
    expect(loginLabel(undefined)).toBe('');
  });
});

describe('knownUsername', () => {
  it('takes the verified person first, then the login, then what the join remembered', () => {
    const login = { ssh_target: 'crew_frank@52.33.141.141' };
    expect(knownUsername({ username: 'frank' }, login, 'other')).toBe('frank');
    expect(knownUsername(null, login, 'other')).toBe('crew_frank');
    expect(knownUsername(null, { ssh_target: 'lab-server' }, 'crew_frank')).toBe('crew_frank');
    expect(knownUsername(null, { ssh_target: 'lab-server' }, null)).toBeNull();
  });
});
