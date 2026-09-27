import { describe, expect, it } from 'vitest';
import { crewStatusCopy } from './copy';
import type { ConnectFailureKind } from './connectFailure';
import {
  CANCELLABLE_RUN_STATUSES,
  CONNECTION_STATUS,
  deriveConnectionStatus,
  deriveCrewScreen,
  runStatusPresentation,
  sentenceCaseStatus,
  transferStatePresentation,
  type ConnectionStatusInput,
  type CrewScreenInput,
} from './crewStatus';

const connected = { status: 'connected' };
const disconnected = { status: 'disconnected' };
const failure = (kind: ConnectFailureKind) => ({ kind });

function status(overrides: Partial<ConnectionStatusInput>) {
  return deriveConnectionStatus({
    connection: connected,
    lastConnectFailure: null,
    inFlight: false,
    verified: false,
    observationError: false,
    notJoined: false,
    ...overrides,
  });
}

describe('deriveConnectionStatus, row by row', () => {
  it('has no status without a selected connection', () => {
    expect(status({ connection: null, verified: true })).toBeNull();
  });

  it.each(['host_key_unknown', 'host_key_changed', 'workspace_identity_mismatch'] as const)(
    'reads "Can’t verify server" for the trust code %s, before anything else',
    (kind) => {
      expect(status({ lastConnectFailure: failure(kind), inFlight: true, verified: true })).toBe(
        'cant-verify'
      );
    }
  );

  it('reads "Connecting…" while a connect or sign-in is in flight', () => {
    expect(status({ inFlight: true, verified: true })).toBe('connecting');
    expect(status({ connection: disconnected, inFlight: true })).toBe('connecting');
  });

  it('reads "Connected" only with a verified snapshot and observed privacy', () => {
    expect(status({ verified: true })).toBe('connected');
    expect(status({ verified: true, observationError: true })).toBe('connected');
  });

  it('reads "Updates unavailable" for a saved-connected connection with an observation error', () => {
    expect(status({ observationError: true })).toBe('updates-unavailable');
  });

  it('reads "Checking connection" for a saved-connected connection with no snapshot yet', () => {
    expect(status({})).toBe('checking');
    expect(status({ reverifying: false })).toBe('checking');
  });

  it('reads a neutral "Updating…" while a verified view is observed again by itself', () => {
    expect(status({ reverifying: true })).toBe('updating');
    expect(CONNECTION_STATUS.updating).toMatchObject({
      tone: 'neutral',
      word: crewStatusCopy.updating,
      spinner: false,
    });
    // Once it gave up, it says so; a verified view, a join or an offline connection still win.
    expect(status({ reverifying: true, observationError: true })).toBe('updates-unavailable');
    expect(status({ reverifying: true, verified: true })).toBe('connected');
    expect(status({ reverifying: true, notJoined: true })).toBe('not-joined');
    expect(status({ reverifying: true, connection: disconnected })).toBe('offline');
  });

  it('reads "Reconnecting…" while Crew picks a dropped view up again (Q2-01)', () => {
    expect(CONNECTION_STATUS.reconnecting).toMatchObject({
      tone: 'neutral',
      word: crewStatusCopy.reconnecting,
      spinner: true,
    });
    expect(crewStatusCopy.reconnecting).toBe('Reconnecting…');
    // Whatever the saved record says meanwhile, and while its connect is in flight.
    expect(status({ reconnecting: true })).toBe('reconnecting');
    expect(status({ reconnecting: true, connection: disconnected })).toBe('reconnecting');
    expect(status({ reconnecting: true, inFlight: true })).toBe('reconnecting');
    expect(status({ reconnecting: true, reverifying: true })).toBe('reconnecting');
    // A verified view or a trust failure still wins.
    expect(status({ reconnecting: true, verified: true })).toBe('connected');
    expect(status({ reconnecting: true, lastConnectFailure: failure('host_key_changed') })).toBe(
      'cant-verify'
    );
  });

  it('never reads "Updates unavailable" while offline or reconnecting: Reconnecting… > Offline > Updates unavailable (Q2-17)', () => {
    expect(status({ reconnecting: true, observationError: true })).toBe('reconnecting');
    expect(status({ reconnecting: true, connection: disconnected, observationError: true })).toBe(
      'reconnecting'
    );
    expect(status({ connection: disconnected, observationError: true })).toBe('offline');
    expect(status({ observationError: true })).toBe('updates-unavailable');
  });

  it('reads "Sign-in needed" after a connect that failed with crew_ssh_auth_required', () => {
    expect(status({ connection: disconnected, lastConnectFailure: failure('auth_required') })).toBe(
      'sign-in-needed'
    );
  });

  it.each(['bridge_missing', 'handoff_failed'] as const)(
    'reads "Not set up on this server" after %s',
    (kind) => {
      expect(status({ connection: disconnected, lastConnectFailure: failure(kind) })).toBe(
        'not-set-up'
      );
    }
  );

  it('reads "Not joined yet" for a connected person who is not a member, even while unverified', () => {
    expect(status({ notJoined: true })).toBe('not-joined');
    expect(status({ notJoined: true, observationError: true })).toBe('not-joined');
    expect(status({ connection: disconnected, notJoined: true })).toBe('not-joined');
    // A verified snapshot proves membership.
    expect(status({ notJoined: true, verified: true })).toBe('connected');
  });

  it('reads "Offline" for a saved-disconnected connection with no failure (after an app restart)', () => {
    expect(status({ connection: disconnected })).toBe('offline');
    expect(status({ connection: disconnected, observationError: true })).toBe('offline');
  });

  it.each(['unreachable', 'ssh_failed', 'unknown'] as const)(
    'reads "Can’t connect" for anything else that failed (%s)',
    (kind) => {
      expect(status({ connection: disconnected, lastConnectFailure: failure(kind) })).toBe(
        'cant-connect'
      );
    }
  );

  it('keeps the pinned words and puts the pinned verified sentence in the screen-reader text', () => {
    expect(CONNECTION_STATUS.checking.word).toBe('Checking connection');
    expect(CONNECTION_STATUS['updates-unavailable'].word).toBe('Updates unavailable');
    expect(CONNECTION_STATUS.connected).toMatchObject({
      word: 'Connected',
      tone: 'success',
      srText: 'Connected · identity verified',
    });
    expect(CONNECTION_STATUS.connecting).toMatchObject({ tone: 'neutral', spinner: true });
    expect(CONNECTION_STATUS['sign-in-needed']).toMatchObject({ tone: 'warning' });
    expect(CONNECTION_STATUS['cant-verify']).toMatchObject({ tone: 'danger' });
    expect(CONNECTION_STATUS['not-set-up']).toMatchObject({ tone: 'danger' });
    expect(CONNECTION_STATUS['not-joined']).toMatchObject({ tone: 'neutral' });
    expect(CONNECTION_STATUS.offline).toMatchObject({ tone: 'idle', word: crewStatusCopy.offline });
    expect(CONNECTION_STATUS['cant-connect']).toMatchObject({ tone: 'danger' });
  });
});

const workspace = {
  teams: [{ id: 'team-1' }],
  channels: [
    { id: 'channel-1', team_id: 'team-1' },
    { id: 'channel-2', team_id: 'team-1' },
  ],
};

function screen(overrides: Partial<CrewScreenInput>) {
  return deriveCrewScreen({
    connectionsState: 'loaded',
    connectionCount: 1,
    connection: connected,
    lastConnectFailure: null,
    inFlight: false,
    signInOpen: false,
    view: null,
    channelId: '',
    observationError: false,
    notJoined: false,
    ...overrides,
  });
}

describe('deriveCrewScreen, row by row', () => {
  it('is loading while the first connections request is in flight', () => {
    expect(screen({ connectionsState: 'loading', connectionCount: 0, connection: null })).toBe(
      'loading'
    );
  });

  it('is loading, never welcome, while a saved connection is not selected yet', () => {
    expect(screen({ connection: null })).toBe('loading');
  });

  it('is welcome only when the loaded list is empty', () => {
    expect(screen({ connectionCount: 0, connection: null })).toBe('welcome');
  });

  it('shows the connection bar (updates paused), not welcome, when the list could not be loaded', () => {
    expect(screen({ connectionsState: 'failed', connectionCount: 0, connection: null })).toBe(
      'updates-paused'
    );
  });

  it('is connecting while a connect or sign-in runs with nothing verified to show', () => {
    expect(screen({ inFlight: true })).toBe('connecting');
    expect(screen({ connection: disconnected, inFlight: true, signInOpen: true })).toBe(
      'connecting'
    );
  });

  it('is offline for a saved-disconnected connection with no failure code', () => {
    expect(screen({ connection: disconnected })).toBe('offline');
  });

  it('is offline, with the failure in the connection bar, after an unreachable or unclassified failure', () => {
    expect(screen({ connection: disconnected, lastConnectFailure: failure('unreachable') })).toBe(
      'offline'
    );
    expect(screen({ connection: disconnected, lastConnectFailure: failure('unknown') })).toBe(
      'offline'
    );
  });

  it('is sign-in after crew_ssh_auth_required once the dialog is closed', () => {
    expect(screen({ connection: disconnected, lastConnectFailure: failure('auth_required') })).toBe(
      'sign-in'
    );
    expect(
      screen({
        connection: disconnected,
        lastConnectFailure: failure('auth_required'),
        signInOpen: true,
        inFlight: true,
      })
    ).toBe('connecting');
  });

  it.each(['host_key_unknown', 'host_key_changed', 'workspace_identity_mismatch'] as const)(
    'is trust for %s, even over a verified view',
    (kind) => {
      expect(screen({ connection: disconnected, lastConnectFailure: failure(kind) })).toBe('trust');
      expect(
        screen({ lastConnectFailure: failure(kind), view: workspace, channelId: 'channel-1' })
      ).toBe('trust');
    }
  );

  it.each(['bridge_missing', 'handoff_failed'] as const)('is not-set-up after %s', (kind) => {
    expect(screen({ connection: disconnected, lastConnectFailure: failure(kind) })).toBe(
      'not-set-up'
    );
  });

  it('is join for a connected person who is not a member', () => {
    expect(screen({ notJoined: true })).toBe('join');
    expect(screen({ notJoined: true, observationError: true })).toBe('join');
  });

  it('is updates-paused after an observation error with no view left on a connected connection', () => {
    expect(screen({ connection: connected, observationError: true })).toBe('updates-paused');
  });

  it('is offline, not updates-paused, when a saved-disconnected connection’s observer errors', () => {
    expect(screen({ connection: disconnected, observationError: true })).toBe('offline');
  });

  it('agrees with the status row whenever an observation error is present', () => {
    const cases: [{ status: string }, string, string][] = [
      [connected, 'updates-unavailable', 'updates-paused'],
      [disconnected, 'offline', 'offline'],
    ];
    for (const [connection, expectedStatus, expectedScreen] of cases) {
      expect(status({ connection, observationError: true })).toBe(expectedStatus);
      expect(screen({ connection, observationError: true })).toBe(expectedScreen);
    }
  });

  it('is sign-in, not updates-paused, for a connection that needs authentication', () => {
    expect(
      screen({ connection: { status: 'authentication_required' }, observationError: true })
    ).toBe('sign-in');
  });

  it('is connecting, never updates-paused or offline, while a dropped view is picked up again (Q2-01)', () => {
    expect(screen({ reconnecting: true })).toBe('connecting');
    expect(screen({ reconnecting: true, connection: disconnected })).toBe('connecting');
    expect(screen({ reconnecting: true, observationError: true })).toBe('connecting');
    // A trust failure keeps its own screen, and a verified view is drawn as ever.
    expect(screen({ reconnecting: true, lastConnectFailure: failure('host_key_unknown') })).toBe(
      'trust'
    );
    expect(screen({ reconnecting: true, view: workspace, channelId: 'channel-1' })).toBe('channel');
  });

  it('is checking, never welcome or join, while an enrolled connection waits for its first snapshot', () => {
    expect(screen({})).toBe('checking');
  });

  it('is no-team for a verified workspace with no teams', () => {
    expect(screen({ view: { teams: [], channels: [] } })).toBe('no-team');
  });

  it('is no-channel when the selected team has no open channel', () => {
    expect(screen({ view: workspace, channelId: '' })).toBe('no-channel');
  });

  it('is channel for a verified view with the selected channel', () => {
    expect(screen({ view: workspace, channelId: 'channel-2' })).toBe('channel');
  });

  it('keeps the verified (or last verified) workspace through a reconnect, a failed connect or a sign-in', () => {
    const view = { view: workspace, channelId: 'channel-1' };
    expect(screen({ ...view, inFlight: true })).toBe('channel');
    expect(screen({ ...view, notJoined: true })).toBe('channel');
    expect(screen({ ...view, lastConnectFailure: failure('auth_required') })).toBe('channel');
    expect(screen({ ...view, connection: disconnected })).toBe('channel');
  });
});

describe('run status words', () => {
  it.each([
    ['starting', 'Starting…', 'running', null, true],
    ['running', 'Working…', 'running', 'open', true],
    ['waiting_for_approval', 'Waiting for your approval', 'warning', 'review', true],
    ['cancellation_pending', 'Stopping…', 'running', null, true],
    ['cancellation_unconfirmed', 'Stop not confirmed', 'warning', 'stop-again', true],
    ['interrupted', 'Interrupted', 'muted', 'open', true],
    ['outcome_not_durable', 'Outcome unknown', 'muted', 'open', true],
    ['completed', 'Done', 'muted', 'open', false],
    ['failed', 'Couldn’t finish', 'danger', 'open', false],
    ['cancelled', 'Stopped', 'muted', 'open', false],
  ])('%s reads "%s"', (value, word, tone, action, stoppable) => {
    expect(runStatusPresentation(value)).toEqual({ word, tone, action, stoppable });
  });

  it('turns an unknown status into its words, muted, with Open and no Stop', () => {
    expect(runStatusPresentation('awaiting_REMOTE_quota')).toEqual({
      word: 'Awaiting remote quota',
      tone: 'muted',
      action: 'open',
      stoppable: false,
    });
    expect(runStatusPresentation('constructor').word).toBe('Constructor');
  });

  it('offers Stop exactly in the statuses the cancel route accepts', () => {
    expect([...CANCELLABLE_RUN_STATUSES].sort()).toEqual(
      [
        'cancellation_pending',
        'cancellation_unconfirmed',
        'interrupted',
        'outcome_not_durable',
        'running',
        'starting',
        'waiting_for_approval',
      ].sort()
    );
  });

  it('sentence-cases snake case', () => {
    expect(sentenceCaseStatus('needs_file_selection')).toBe('Needs file selection');
    expect(sentenceCaseStatus('')).toBe('');
  });
});

describe('transfer state words', () => {
  const transfer = (
    state: string,
    direction: 'upload' | 'download' = 'upload',
    error?: string
  ) => ({
    state,
    direction,
    offset: 42,
    size: 100,
    error: error ?? null,
  });

  it.each([
    [transfer('starting'), 'starting', 'Starting…', true],
    [transfer('uploading'), 'uploading', 'Uploading 42%', true],
    [transfer('downloading', 'download'), 'downloading', 'Downloading 42%', true],
    [transfer('publishing'), 'finishing', 'Finishing…', true],
    [transfer('pause_requested'), 'pausing', 'Pausing…', true],
    [transfer('needs_file_selection'), 'paused', 'Paused', false],
    [transfer('needs_file_selection', 'upload', 'SSH bridge failed'), 'failed', 'Failed', false],
    [transfer('completed'), 'ready', 'Ready', false],
    [transfer('completed', 'download'), 'saved', 'Saved', false],
    [transfer('failed'), 'failed', 'Failed', false],
    [transfer('publication_unconfirmed'), 'not-confirmed', 'Not confirmed', false],
    [transfer('queued_for_scan'), 'unknown', 'Queued for scan', false],
  ])('%o reads %s', (input, key, word, active) => {
    expect(transferStatePresentation(input)).toMatchObject({ key, word, active });
  });

  it('bounds the percentage and survives an empty file', () => {
    expect(
      transferStatePresentation({ state: 'uploading', direction: 'upload', offset: 7, size: 0 })
        .word
    ).toBe('Uploading 0%');
    expect(
      transferStatePresentation({ state: 'uploading', direction: 'upload', offset: 300, size: 100 })
        .percent
    ).toBe(100);
  });
});
