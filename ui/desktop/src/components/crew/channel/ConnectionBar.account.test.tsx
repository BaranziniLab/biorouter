import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  addedViaText,
  newDevicesCopy,
  rejoinCopy,
  setupJobsCopy,
  versionCopy,
} from '../account/copy';
import { resetBuildBrokerVersionForTests } from '../account/brokerVersion';
import { markNeedsRejoin, needsRejoin, resetRejoinMarksForTests } from '../account/rejoinMarks';
import { CrewHttpError, type CrewConnection } from '../crewApi';
import { resetSetupStartedHereForTests, startSetup } from '../onboarding/setup';
import { fakeConnection, fakeSnapshot, makeCrew, renderWithCrew } from '../onboarding/testCrew';
import type { CrewController } from '../state/types';
import { ConnectionBar } from './ConnectionBar';
import { calendarDate } from './presentation';

/**
 * The connection bar's account-ownership notes (docs/crew/design/account-ownership.md, "Desktop"):
 * Add this computer again, the daemon's new computers with Acknowledge, the older-Crew notice by
 * role (by the server's `server_version` against this build's broker, or by the account capability
 * when the server says no version), and setups a chat started, with Sign in.
 */

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn(), signIn: [] as Record<string, unknown>[] }));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});
vi.mock('../auth/SignInDialog', () => ({
  SignInDialog: (props: Record<string, unknown>) => {
    mocks.signIn.push(props);
    return props.open ? <div data-testid="setup-sign-in" /> : null;
  },
}));

type Route = (body: unknown) => unknown;

/** The broker this build carries (`GET /crew/setup/artifact`). */
const ARTIFACT = {
  version: '1.94.2',
  target: 'x86_64-unknown-linux-gnu',
  sha256: 'd'.repeat(64),
  size: 3556248,
  source: 'bundled',
};

function daemon(routes: Record<string, Route> = {}) {
  mocks.crewHttp.mockImplementation(async (path: string, method = 'GET', body?: unknown) => {
    const route = routes[`${method} ${path}`];
    if (route) return route(body);
    if (path === '/setup') return [];
    return {};
  });
}

const calls = (method: string, path: string) =>
  mocks.crewHttp.mock.calls.filter(
    ([called, calledMethod = 'GET']) => called === path && calledMethod === method
  );

function renderBar(connection: CrewConnection, overrides: Partial<CrewController> = {}) {
  const snapshot = fakeSnapshot();
  const crew = makeCrew({
    connectionId: connection.id,
    connection,
    connections: [connection],
    snapshot,
    observedPrivacy: {
      connectionId: connection.id,
      mode: 'private',
      institutionId: 'ucsf',
      policyEpoch: 1,
    },
    screen: 'channel',
    ...overrides,
  });
  return renderWithCrew(<ConnectionBar />, crew);
}

beforeEach(() => {
  mocks.crewHttp.mockReset();
  mocks.signIn.length = 0;
  resetRejoinMarksForTests();
  resetSetupStartedHereForTests();
  resetBuildBrokerVersionForTests();
  daemon();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('Add this computer again (AO-9)', () => {
  it('is offered on crew_device_not_recognized, and adds this computer with a new key', async () => {
    daemon({
      'POST /connections/conn-1/rejoin': () => ({
        role: 'member',
        status: 'joined',
        admitted: 'device',
        evicted_device: { fingerprint: 'AAAA BBBB CCCC DDDD', added_at: 1 },
      }),
    });
    const rendered = renderBar(fakeConnection(), {
      refreshErrorCode: 'crew_device_not_recognized',
    });
    const note = await screen.findByTestId('crew-rejoin');
    expect(note).toHaveTextContent(rejoinCopy.notRecognized('lab'));
    // One line in view: the daemon's first sentence.
    expect(rejoinCopy.notRecognized('lab')).toBe('lab no longer recognizes this computer.');
    // Its second sentence says more than the button (the way back works only while the account is
    // still a member), so it is behind the info icon beside the line, word for word the daemon's,
    // and not in view at rest (design.md 9.16).
    const help = within(note).getByRole('button', { name: 'About adding this computer again' });
    expect(help).toHaveAccessibleDescription(
      'Choose Add this computer again while you are still a member.'
    );
    expect(within(note).getByText(rejoinCopy.notRecognizedHelp).closest('.sr-only')).not.toBeNull();
    fireEvent.click(within(note).getByRole('button', { name: rejoinCopy.action }));
    await waitFor(() => expect(calls('POST', '/connections/conn-1/rejoin')).toHaveLength(1));
    // Connected already: no second connect.
    expect(rendered.crew().connect).not.toHaveBeenCalled();
    await waitFor(() => expect(rendered.crew().refresh).toHaveBeenCalled());
    rendered.update({ refreshErrorCode: null });
    const done = await screen.findByTestId('crew-rejoined');
    // Back in, and the computer removed to make room, in view; that chats need access again is
    // behind Details, word for word the command line's (design.md 9.16).
    expect(within(done).getByText('This computer is back in lab.')).toBeVisible();
    expect(within(done).getByText(rejoinCopy.evicted('AAAA BBBB CCCC DDDD'))).toBeVisible();
    expect(done).not.toHaveTextContent(/need access again/);
    fireEvent.click(within(done).getByRole('button', { name: 'Details' }));
    expect(
      within(done).getByText('Chats that had Crew access through it need access again.')
    ).toBeVisible();
    expect(rejoinCopy.done('lab')).toBe(
      'This computer is back in lab. Chats that had Crew access through it need access again.'
    );
  });

  it('connects first when the connection is down, then adds this computer again', async () => {
    daemon({ 'POST /connections/conn-1/rejoin': () => ({ role: 'host', status: 'joined' }) });
    const rendered = renderBar(
      fakeConnection({ status: 'disconnected', last_error_code: 'crew_device_key_missing' })
    );
    const note = await screen.findByTestId('crew-rejoin');
    expect(note).toHaveTextContent(rejoinCopy.keyMissing('lab'));
    expect(rejoinCopy.keyMissing('lab')).toBe('This computer no longer has its key for lab.');
    // The daemon's second sentence here only names the button, so nothing is folded beside it.
    expect(within(note).queryByTestId('crew-rejoin-help')).toBeNull();
    fireEvent.click(within(note).getByRole('button', { name: rejoinCopy.action }));
    await waitFor(() => expect(calls('POST', '/connections/conn-1/rejoin')).toHaveLength(1));
    expect(rendered.crew().connect).toHaveBeenCalledWith({ userInitiated: true });
  });

  it('is offered for a computer the person removed in Keys, and clears once it is back', async () => {
    daemon({ 'POST /connections/conn-1/rejoin': () => ({ role: 'member', status: 'joined' }) });
    markNeedsRejoin('conn-1');
    renderBar(fakeConnection());
    const note = await screen.findByTestId('crew-rejoin');
    expect(note).toHaveTextContent(rejoinCopy.removedHere('lab'));
    expect(rejoinCopy.removedHere('lab')).toBe('You removed this computer from lab.');
    expect(within(note).queryByTestId('crew-rejoin-help')).toBeNull();
    fireEvent.click(within(note).getByRole('button', { name: rejoinCopy.action }));
    await waitFor(() => expect(needsRejoin('conn-1')).toBe(false));
    await waitFor(() => expect(screen.queryByTestId('crew-rejoin')).toBeNull());
  });

  it('says why when the workspace refuses, and keeps the way to try again', async () => {
    daemon({
      'POST /connections/conn-1/rejoin': () => {
        throw new CrewHttpError(
          'You were removed from lab. Ask a member to invite you again.',
          409,
          'crew_join_removed'
        );
      },
    });
    renderBar(fakeConnection(), { refreshErrorCode: 'crew_device_not_recognized' });
    const note = await screen.findByTestId('crew-rejoin');
    fireEvent.click(within(note).getByRole('button', { name: rejoinCopy.action }));
    await waitFor(() =>
      expect(screen.getByTestId('crew-rejoin')).toHaveTextContent(
        'You were removed from lab. Ask a member to invite you again.'
      )
    );
    expect(
      within(screen.getByTestId('crew-rejoin')).getByRole('button', { name: rejoinCopy.action })
    ).toBeInTheDocument();
  });

  it('says it once, in place of the error that carried the code', async () => {
    renderBar(fakeConnection(), {
      error: {
        message:
          'lab no longer recognizes this computer. Choose Add this computer again while you are still a member.',
        code: 'crew_device_not_recognized',
        source: 'global',
      },
      errorSlotFor: vi.fn((source: string) => source === 'global'),
    });
    await screen.findByTestId('crew-rejoin');
    expect(screen.getAllByText(/no longer recognizes this computer/)).toHaveLength(1);
  });
});

describe('new computers on the account (AO-10)', () => {
  it('names a device added by signing in, and acknowledges it on the daemon', async () => {
    daemon({ 'POST /connections/conn-1/devices/acknowledge': () => null });
    const rendered = renderBar(
      fakeConnection({
        new_devices: [{ fingerprint: 'AAAA BBBB CCCC DDDD', added_at: null, added_via: 'account' }],
      })
    );
    const note = await screen.findByTestId('crew-new-devices');
    // Which computer and how it was added: its fingerprint, as Keys lists it, in the sentence
    // `biorouter crew status` says.
    expect(note).toHaveTextContent(
      newDevicesCopy.one('AAAA BBBB CCCC DDDD', null, addedViaText('account'))
    );
    expect(within(note).getByText(/^New computer on your account:/)).toHaveTextContent(
      /^New computer on your account: AAAA BBBB CCCC DDDD, added by signing in to the server\.$/
    );
    fireEvent.click(within(note).getByRole('button', { name: newDevicesCopy.reviewName }));
    expect(rendered.crew().openDialog).toHaveBeenCalledWith({ kind: 'keys' });
    fireEvent.click(within(note).getByRole('button', { name: newDevicesCopy.acknowledge }));
    await waitFor(() =>
      expect(calls('POST', '/connections/conn-1/devices/acknowledge')).toHaveLength(1)
    );
    // The route requires the fingerprints the notice showed, and acknowledges only those.
    expect(calls('POST', '/connections/conn-1/devices/acknowledge')[0][2]).toEqual({
      fingerprints: ['AAAA BBBB CCCC DDDD'],
    });
    await waitFor(() => expect(screen.queryByTestId('crew-new-devices')).toBeNull());
  });

  it('says how any other computer was added in Keys’ words, and nothing for a way it does not know', async () => {
    const connection = fakeConnection({
      new_devices: [{ fingerprint: 'BBBB', added_at: 1_700_000_100, added_via: 'token' }],
    });
    const rendered = renderBar(connection);
    expect(await screen.findByTestId('crew-new-devices')).toHaveTextContent(
      `New computer on your account: BBBB, added ${calendarDate(1_700_000_100)} with an invitation token.`
    );
    const unknown = fakeConnection({
      new_devices: [{ fingerprint: 'CCCC', added_at: null, added_via: 'carrier_pigeon' }],
    });
    rendered.update({ connection: unknown, connections: [unknown] });
    await waitFor(() =>
      expect(
        within(screen.getByTestId('crew-new-devices')).getByText(/^New computer on your account:/)
      ).toHaveTextContent(/^New computer on your account: CCCC\.$/)
    );
  });

  it('acknowledges every device it counted, and only those', async () => {
    daemon({ 'POST /connections/conn-1/devices/acknowledge': () => null });
    renderBar(
      fakeConnection({
        new_devices: [
          { fingerprint: 'AAAA', added_at: 1_700_000_000, added_via: 'account' },
          { fingerprint: 'BBBB', added_at: 1_700_000_100, added_via: 'token' },
        ],
      })
    );
    const note = await screen.findByTestId('crew-new-devices');
    fireEvent.click(within(note).getByRole('button', { name: newDevicesCopy.acknowledge }));
    await waitFor(() =>
      expect(calls('POST', '/connections/conn-1/devices/acknowledge')).toHaveLength(1)
    );
    expect(calls('POST', '/connections/conn-1/devices/acknowledge')[0][2]).toEqual({
      fingerprints: ['AAAA', 'BBBB'],
    });
  });

  it('says a failed Acknowledge under its own workspace only (finding 41)', async () => {
    daemon({
      'POST /connections/conn-1/devices/acknowledge': () => {
        throw new CrewHttpError('Not now.', 503, 'crew_not_sent');
      },
    });
    const first = fakeConnection({
      new_devices: [{ fingerprint: 'AAAA', added_at: null, added_via: 'account' }],
    });
    const rendered = renderBar(first);
    fireEvent.click(
      within(await screen.findByTestId('crew-new-devices')).getByRole('button', {
        name: newDevicesCopy.acknowledge,
      })
    );
    expect(await screen.findByText(newDevicesCopy.acknowledgeFailed)).toBeInTheDocument();
    const second = fakeConnection({
      id: 'conn-2',
      new_devices: [{ fingerprint: 'BBBB', added_at: null, added_via: 'account' }],
    });
    rendered.update({
      connectionId: 'conn-2',
      connection: second,
      connections: [first, second],
      observedPrivacy: {
        connectionId: 'conn-2',
        mode: 'private',
        institutionId: 'ucsf',
        policyEpoch: 1,
      },
    });
    expect(await screen.findByTestId('crew-new-devices')).toHaveTextContent(
      newDevicesCopy.one('BBBB', null, addedViaText('account'))
    );
    expect(screen.queryByText(newDevicesCopy.acknowledgeFailed)).toBeNull();
  });

  it('puts focus back on the channel once Acknowledge took its note away, even drawn late', async () => {
    daemon({ 'POST /connections/conn-1/devices/acknowledge': () => null });
    // The channel heading is drawn again a moment after the connections reload, as the main area
    // can be while it reads them.
    let showHeading: () => void = () => {};
    let hideHeading: () => void = () => {};
    function Stage() {
      const [shown, setShown] = useState(true);
      showHeading = () => setShown(true);
      hideHeading = () => setShown(false);
      return (
        <div className="crew-app">
          {shown ? (
            <h1>
              <button type="button">general</button>
            </h1>
          ) : null}
          <ConnectionBar />
        </div>
      );
    }
    const connection = fakeConnection({
      new_devices: [{ fingerprint: 'AAAA', added_at: null, added_via: 'account' }],
    });
    renderWithCrew(
      <Stage />,
      makeCrew({
        connectionId: connection.id,
        connection,
        connections: [connection],
        snapshot: fakeSnapshot(),
        observedPrivacy: {
          connectionId: connection.id,
          mode: 'private',
          institutionId: 'ucsf',
          policyEpoch: 1,
        },
        screen: 'channel',
        reloadConnections: vi.fn(() => {
          hideHeading();
          window.setTimeout(() => act(() => showHeading()), 120);
        }),
      })
    );
    const acknowledge = within(await screen.findByTestId('crew-new-devices')).getByRole('button', {
      name: newDevicesCopy.acknowledge,
    });
    acknowledge.focus();
    fireEvent.click(acknowledge);
    await waitFor(() => expect(screen.queryByTestId('crew-new-devices')).toBeNull());
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole('button', { name: 'general' }))
    );
  });

  it('shows a computer added while the Crew view is open, from the next verified view', async () => {
    const own = { fingerprint: '1111 1111 1111 1111', added_at: 1, added_via: 'invitation' };
    const added = { fingerprint: 'AAAA BBBB CCCC DDDD', added_at: 2, added_via: 'account' };
    const view = (devices: (typeof own)[]) =>
      fakeSnapshot({ actor: { ...fakeSnapshot().actor, devices } });
    const reloadConnections = vi.fn();
    const rendered = renderBar(fakeConnection(), {
      snapshot: view([own]),
      reloadConnections,
    });
    await waitFor(() => expect(calls('GET', '/setup')).toHaveLength(1));
    // A view arrives every few seconds: the same computers again read nothing.
    rendered.update({ snapshot: view([own]) });
    expect(reloadConnections).not.toHaveBeenCalled();
    expect(screen.queryByTestId('crew-new-devices')).toBeNull();
    // The person's new computer signed in: the next view lists it, and the connections are read
    // again, once, without leaving the view (live QA: only after going Home and back).
    rendered.update({ snapshot: view([own, added]) });
    await waitFor(() => expect(reloadConnections).toHaveBeenCalledTimes(1));
    const connection = fakeConnection({ new_devices: [added] });
    rendered.update({ connection, connections: [connection], snapshot: view([own, added]) });
    expect(await screen.findByTestId('crew-new-devices')).toHaveTextContent(
      `New computer on your account: AAAA BBBB CCCC DDDD, added ${calendarDate(added.added_at)} by signing in to the server.`
    );
    expect(reloadConnections).toHaveBeenCalledTimes(1);
  });

  it('counts several', async () => {
    renderBar(
      fakeConnection({
        new_devices: [
          { fingerprint: 'AAAA', added_at: 1_700_000_000, added_via: 'account' },
          { fingerprint: 'BBBB', added_at: 1_700_000_100, added_via: 'token' },
        ],
      })
    );
    expect(await screen.findByTestId('crew-new-devices')).toHaveTextContent(newDevicesCopy.many(2));
  });

  it('says nothing when the daemon lists none', async () => {
    renderBar(fakeConnection());
    await waitFor(() => expect(calls('GET', '/setup')).toHaveLength(1));
    expect(screen.queryByTestId('crew-new-devices')).toBeNull();
  });
});

describe('a server that runs an older Crew (AO-4, AO-9)', () => {
  it('offers the host Update Crew, which looks the server up with the saved login', async () => {
    const connection = fakeConnection({ account_identity: false, port: 2222 });
    const rendered = renderBar(connection, { isHost: true });
    const note = await screen.findByTestId('crew-older-server');
    expect(note).toHaveTextContent(versionCopy.older('lab'));
    fireEvent.click(within(note).getByRole('button', { name: versionCopy.update }));
    expect(rendered.crew().openDialog).toHaveBeenCalledWith({
      kind: 'open-workspace',
      route: {
        ssh_target: 'bob@hpc.ucsf.edu',
        port: 2222,
        identity_file: null,
        proxy_jump: null,
      },
      check: true,
    });
  });

  it('tells everyone else whom to ask', async () => {
    renderBar(fakeConnection({ account_identity: false }), { isHost: false });
    const note = await screen.findByTestId('crew-older-server');
    expect(note).toHaveTextContent(`${versionCopy.older('lab')} ${versionCopy.askHost('@alice')}`);
    expect(within(note).queryByRole('button', { name: versionCopy.update })).toBeNull();
    fireEvent.click(within(note).getByRole('button', { name: versionCopy.dismiss }));
    await waitFor(() => expect(screen.queryByTestId('crew-older-server')).toBeNull());
  });

  it('says nothing for a server that belongs to accounts, or one not verified yet', async () => {
    renderBar(fakeConnection({ account_identity: true }));
    await waitFor(() => expect(calls('GET', '/setup')).toHaveLength(1));
    expect(screen.queryByTestId('crew-older-server')).toBeNull();
  });

  it('decides on the account capability alone, and reads no version, when the server says none', async () => {
    daemon({ 'GET /setup/artifact': () => ARTIFACT });
    renderBar(fakeConnection({ account_identity: false, server_version: null }));
    await screen.findByTestId('crew-older-server');
    expect(calls('GET', '/setup/artifact')).toHaveLength(0);
  });

  describe('by the version the server said', () => {
    it('says it for a server older than this build, capability or not', async () => {
      daemon({ 'GET /setup/artifact': () => ARTIFACT });
      renderBar(fakeConnection({ account_identity: true, server_version: '1.94.1' }), {
        isHost: true,
      });
      const note = await screen.findByTestId('crew-older-server');
      expect(note).toHaveTextContent(versionCopy.older('lab'));
      expect(within(note).getByRole('button', { name: versionCopy.update })).toBeInTheDocument();
    });

    it('tells a member whom to ask', async () => {
      daemon({ 'GET /setup/artifact': () => ARTIFACT });
      renderBar(fakeConnection({ account_identity: true, server_version: '1.93.9-dev' }), {
        isHost: false,
      });
      expect(await screen.findByTestId('crew-older-server')).toHaveTextContent(
        `${versionCopy.older('lab')} ${versionCopy.askHost('@alice')}`
      );
    });

    it.each([
      ['as new as this build', '1.94.2'],
      ['newer than this build', '1.95.0'],
    ])('says nothing for a server %s, even without the capability', async (_, version) => {
      daemon({ 'GET /setup/artifact': () => ARTIFACT });
      renderBar(fakeConnection({ account_identity: false, server_version: version }));
      await waitFor(() => expect(calls('GET', '/setup/artifact')).toHaveLength(1));
      await waitFor(() => expect(calls('GET', '/setup')).toHaveLength(1));
      expect(screen.queryByTestId('crew-older-server')).toBeNull();
    });

    it('says nothing while this build carries no Crew to compare with', async () => {
      daemon({
        'GET /setup/artifact': () => {
          throw new CrewHttpError(
            'This build has no Crew server program.',
            409,
            'crew_install_artifact_unavailable'
          );
        },
      });
      renderBar(fakeConnection({ account_identity: false, server_version: '1.0.0' }));
      await waitFor(() => expect(calls('GET', '/setup/artifact')).toHaveLength(1));
      await waitFor(() => expect(calls('GET', '/setup')).toHaveLength(1));
      expect(screen.queryByTestId('crew-older-server')).toBeNull();
    });

    it('asks nothing for a server that is not connected', async () => {
      daemon({ 'GET /setup/artifact': () => ARTIFACT });
      renderBar(fakeConnection({ status: 'disconnected', server_version: '1.0.0' }));
      await waitFor(() => expect(calls('GET', '/setup')).toHaveLength(1));
      expect(calls('GET', '/setup/artifact')).toHaveLength(0);
      expect(screen.queryByTestId('crew-older-server')).toBeNull();
    });

    it('reads this build’s version once, and asks again only after a failed read', async () => {
      let fail = true;
      daemon({
        'GET /setup/artifact': () => {
          if (fail) throw new CrewHttpError('Not now.', 503, 'crew_not_sent');
          return ARTIFACT;
        },
      });
      const first = renderBar(fakeConnection({ server_version: '1.94.1' }));
      await waitFor(() => expect(calls('GET', '/setup/artifact')).toHaveLength(1));
      expect(screen.queryByTestId('crew-older-server')).toBeNull();
      first.unmount();

      fail = false;
      const second = renderBar(fakeConnection({ server_version: '1.94.1' }));
      await screen.findByTestId('crew-older-server');
      expect(calls('GET', '/setup/artifact')).toHaveLength(2);
      second.unmount();

      renderBar(fakeConnection({ server_version: '1.94.1' }));
      await screen.findByTestId('crew-older-server');
      expect(calls('GET', '/setup/artifact')).toHaveLength(2);
    });
  });
});

describe('setups a chat started (AO-8, AO-12)', () => {
  const job = (overrides: Record<string, unknown> = {}) => ({
    job_id: 'job-chat',
    intent: 'install',
    state: 'needs_sign_in',
    stage: 'sign_in',
    stages: [{ name: 'sign_in', state: 'running' }],
    server: { login: 'wanjun@leo.ucsf.edu', user: 'wanjun' },
    outcome: null,
    error: null,
    ...overrides,
  });

  it('shows one waiting for the person with its Sign in button', async () => {
    daemon({ 'GET /setup': () => [job()] });
    renderBar(fakeConnection());
    const note = await screen.findByTestId('crew-outside-setup');
    expect(note).toHaveTextContent(setupJobsCopy.needsSignIn('wanjun@leo.ucsf.edu'));
    fireEvent.click(
      within(note).getByRole('button', { name: setupJobsCopy.signInLabel('wanjun@leo.ucsf.edu') })
    );
    expect(await screen.findByTestId('setup-sign-in')).toBeInTheDocument();
    expect(mocks.signIn[mocks.signIn.length - 1]).toMatchObject({
      target: { kind: 'setup', jobId: 'job-chat' },
    });
  });

  it('leaves out a job this window started, and one that ended', async () => {
    daemon({
      'POST /setup': () => job({ job_id: 'job-here' }),
      'GET /setup': () => [
        job({ job_id: 'job-here' }),
        job({ job_id: 'job-done', state: 'finished', stage: 'done' }),
      ],
    });
    await startSetup({ target: { kind: 'connection', connection_id: 'conn-1' }, intent: 'member' });
    renderBar(fakeConnection());
    await waitFor(() => expect(calls('GET', '/setup')).toHaveLength(1));
    expect(screen.queryByTestId('crew-outside-setup')).toBeNull();
  });
});
