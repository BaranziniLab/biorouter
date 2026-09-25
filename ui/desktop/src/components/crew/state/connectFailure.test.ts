import { describe, expect, it } from 'vitest';
import { CrewHttpError } from '../crewApi';
import {
  arrivalConnectDecision,
  CONNECT_FAILURE_CODES,
  classifyConnectFailure,
  isMembershipEnded,
  isNotSetUpFailure,
  isTrustFailure,
  MEMBERSHIP_ENDED_CODE,
  TRUST_FAILURE_KINDS,
  type ArrivalConnectInput,
  type ConnectFailureKind,
} from './connectFailure';

// Today's daemon text for an SSH child that ended before the bridge answered.
const sshText = (status: string) =>
  `Crew SSH failure [ssh_eof; child_before_cleanup=${status}]: SSH ended before Crew answered; reconnect. Submitted operation outcome may be unknown; inspect history before retrying`;

describe('classifyConnectFailure', () => {
  it.each([
    ['crew_ssh_auth_required', 'auth_required'],
    ['crew_ssh_host_key_unknown', 'host_key_unknown'],
    ['crew_ssh_host_key_changed', 'host_key_changed'],
    ['crew_ssh_unreachable', 'unreachable'],
    ['crew_bridge_missing', 'bridge_missing'],
    ['crew_handoff_failed', 'handoff_failed'],
    ['crew_workspace_identity_mismatch', 'workspace_identity_mismatch'],
    ['crew_ssh_failed', 'ssh_failed'],
  ])('maps the daemon code %s to %s and keeps the text unchanged', (code, kind) => {
    const failure = new CrewHttpError('The daemon said so.', 400, code);
    expect(classifyConnectFailure(failure)).toEqual({ kind, code, message: 'The daemon said so.' });
  });

  it('lets a typed code decide even when the text says otherwise', () => {
    expect(
      classifyConnectFailure(new CrewHttpError(sshText('exit_127'), 400, 'crew_ssh_unreachable'))
        .kind
    ).toBe('unreachable');
  });

  it('carries the bounded detail for "Copy details" when the error has one', () => {
    const failure = Object.assign(
      new CrewHttpError('Host key verification failed.', 400, 'crew_ssh_host_key_unknown'),
      { detail: 'The ECDSA host key for hpc offered SHA256:abc' }
    );
    expect(classifyConnectFailure(failure).detail).toBe(
      'The ECDSA host key for hpc offered SHA256:abc'
    );
    expect(
      classifyConnectFailure(new CrewHttpError('x', 400, 'crew_ssh_failed'))
    ).not.toHaveProperty('detail');
  });

  describe('the fallback for a daemon without SSH codes', () => {
    it.each([
      ['no code', undefined],
      ['the generic refusal code', 'crew_request_refused'],
    ])('reads ssh_eof / exit_255 as sign-in needed (%s)', (_label, code) => {
      expect(classifyConnectFailure(new CrewHttpError(sshText('exit_255'), 400, code)).kind).toBe(
        'auth_required'
      );
      expect(classifyConnectFailure(new Error('ssh_eof')).kind).toBe('auth_required');
      expect(classifyConnectFailure(new Error('child exited: exit_255')).kind).toBe(
        'auth_required'
      );
    });

    it('reads exit_127 as Crew not set up, before the end-of-file that accompanies it', () => {
      expect(
        classifyConnectFailure(new CrewHttpError(sshText('exit_127'), 400, 'crew_request_refused'))
          .kind
      ).toBe('bridge_missing');
      expect(classifyConnectFailure(new Error('exit_127')).kind).toBe('bridge_missing');
    });

    it('labels nothing else a host-key or trust problem', () => {
      for (const text of [
        'Host key verification failed.',
        'REMOTE HOST IDENTIFICATION HAS CHANGED',
        'SSH authentication failed for host hpc: bad key',
        'Could not resolve hostname hpc',
        'The workspace key does not match.',
        'child ended with exit_1270',
      ]) {
        const classified = classifyConnectFailure(new CrewHttpError(text, 400));
        expect(classified.kind).toBe('unknown');
        expect(isTrustFailure(classified.kind)).toBe(false);
        expect(classified.message).toBe(text);
      }
    });

    it('treats an unknown code as unknown, not as a prototype key', () => {
      expect(classifyConnectFailure(new CrewHttpError('x', 400, 'constructor')).kind).toBe(
        'unknown'
      );
      expect(classifyConnectFailure(new CrewHttpError('x', 400, 'toString')).kind).toBe('unknown');
    });
  });

  it('gives a non-Error failure the action fallback text', () => {
    expect(classifyConnectFailure('boom')).toEqual({
      kind: 'unknown',
      message: 'Crew could not complete that action.',
    });
  });

  it('marks exactly the three trust kinds and the two setup kinds', () => {
    expect([...TRUST_FAILURE_KINDS].sort()).toEqual(
      ['host_key_changed', 'host_key_unknown', 'workspace_identity_mismatch'].sort()
    );
    const kinds = Object.values(CONNECT_FAILURE_CODES);
    expect(kinds.filter(isTrustFailure).sort()).toEqual([...TRUST_FAILURE_KINDS].sort());
    expect(kinds.filter(isNotSetUpFailure).sort()).toEqual(['bridge_missing', 'handoff_failed']);
    expect(isTrustFailure(undefined)).toBe(false);
  });
});

describe('isMembershipEnded (Q3-12, Q3-50)', () => {
  it('reads only the daemon’s typed code', () => {
    expect(MEMBERSHIP_ENDED_CODE).toBe('crew_membership_ended');
    expect(isMembershipEnded({ last_error_code: 'crew_membership_ended' })).toBe(true);
    expect(isMembershipEnded({ last_error_code: 'crew_ssh_unreachable' })).toBe(false);
    expect(isMembershipEnded({})).toBe(false);
    expect(isMembershipEnded(null)).toBe(false);
    expect(isMembershipEnded(undefined)).toBe(false);
  });
});

describe('arrivalConnectDecision (Q3-08, SECURITY-SENSITIVE)', () => {
  const base: ArrivalConnectInput = {
    connection: { status: 'disconnected' },
    lastConnectFailure: null,
    connecting: false,
    signInPending: false,
  };
  const decide = (overrides: Partial<ArrivalConnectInput>) =>
    arrivalConnectDecision({ ...base, ...overrides });

  it('connects a saved connection the daemon calls disconnected, and nothing else', () => {
    expect(decide({})).toBe('connect');
    for (const status of ['connected', 'error', 'authentication_required'])
      expect(decide({ connection: { status } })).toBe('skip');
    expect(decide({ connection: null })).toBe('skip');
  });

  it('waits while a connect already runs', () => {
    expect(decide({ connecting: true })).toBe('wait');
  });

  it('never connects after a final answer: trust, sign-in, or an ended membership', () => {
    for (const kind of TRUST_FAILURE_KINDS)
      expect(decide({ lastConnectFailure: { kind } })).toBe('skip');
    expect(decide({ lastConnectFailure: { kind: 'auth_required' } })).toBe('skip');
    expect(decide({ signInPending: true })).toBe('skip');
    expect(
      decide({
        connection: { status: 'disconnected', last_error_code: MEMBERSHIP_ENDED_CODE },
      })
    ).toBe('skip');
    // Final even while a connect runs: nothing is left to wait for.
    expect(
      decide({
        connecting: true,
        connection: { status: 'disconnected', last_error_code: MEMBERSHIP_ENDED_CODE },
      })
    ).toBe('skip');
  });

  it('still connects after a failure a person may retry', () => {
    const retryable: ConnectFailureKind[] = [
      'unreachable',
      'ssh_failed',
      'unknown',
      'bridge_missing',
      'handoff_failed',
    ];
    for (const kind of retryable) expect(decide({ lastConnectFailure: { kind } })).toBe('connect');
  });
});
