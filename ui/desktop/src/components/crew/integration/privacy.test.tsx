import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError, type CrewConnection } from '../crewApi';
import { crewObservationCopy } from '../state/copy';
import { connectionUpdateBody } from '../state/useCrewConnections';
import { REOBSERVE_BACKOFF_MS } from '../state/useCrewObservation';
import { installResizeObserverStub, workspaceAction } from '../test/crewTestUtils';
import {
  channelReady,
  connection,
  currentCrew,
  installDaemon,
  mocked,
  renderCrew,
  renderedStatuses,
  richSnapshot,
  type ScriptedDaemon,
} from './harness';

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
// Stable across renders, as the real context's callbacks are.
const config = vi.hoisted(() => ({
  getProviders: async () => [],
  read: async () => '',
  getProviderModels: async () => [],
}));
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return { ...actual, useConfig: () => config };
});
vi.mock('../CrewAuthentication', () => ({ default: () => <div /> }));

installResizeObserverStub();

/** The workspace's own name: the phrase a typed confirmation asks for, and the switcher's name. */
const WORKSPACE = 'lab';

/** Every PATCH of the saved connection, by body. */
function patches(): unknown[] {
  return mocked.crewHttp.mock.calls
    .filter(([path, method]) => path === `/connections/${connection.id}` && method === 'PATCH')
    .map(([, , body]) => body);
}

function policySets(): unknown[] {
  return mocked.crewRequest.mock.calls
    .filter(([, method]) => method === 'policy.set')
    .map(([, , params]) => params);
}

/** A daemon that saves a PATCH and answers with the saved record, as the real one does. */
function savingDaemon(initial: Parameters<typeof installDaemon>[0] = {}): ScriptedDaemon {
  const daemon = installDaemon(initial);
  daemon.state.http = (path, method, body) =>
    path === `/connections/${connection.id}` && method === 'PATCH'
      ? { ...connection, ...(body as object) }
      : undefined;
  return daemon;
}

/** The typed confirmation for making the connection Public: an `alertdialog`, as every one is. */
async function makePublicConfirmation() {
  return screen.findByRole('alertdialog', { name: `Make your ${WORKSPACE} connection public?` });
}

/** Neither a confirmation nor the popover (a Radix `dialog`) is still open. */
function expectNothingOpen() {
  expect(screen.queryByRole('alertdialog')).toBeNull();
  expect(screen.queryByRole('dialog')).toBeNull();
}

/** A member's sentences, which the host must never read in Workspace settings (SF2-N7). */
const MEMBER_WORDS = [/Only the host/, /can invite new people/, /No one else has joined/];

/** The full record the daemon keeps (L18), with only the mode changed. */
function fullBody(mode: 'private' | 'public', from: CrewConnection = connection) {
  return { ...connectionUpdateBody(from), mode };
}

describe('privacy changes: exposing asks first, the reverse is one click', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('offers no downgrade from the status-row popover where the workspace is Private for everyone (Q3-54)', async () => {
    savingDaemon();
    renderCrew();
    await channelReady();
    const user = userEvent.setup();

    // Its own description said the models that can read the workspace "stay the same": a control
    // that changed nothing, offered first. Privacy… is the one action, and leads to the settings.
    await user.click(screen.getByRole('button', { name: /^Privacy: Private · ucsf/ }));
    const popover = await screen.findByRole('dialog', { name: /^Privacy: Private/ });
    expect(
      within(popover).queryByRole('button', { name: 'Make my connection public…' })
    ).toBeNull();
    expect(
      within(popover)
        .getAllByRole('button')
        .map((button) => button.textContent)
    ).toEqual(['Privacy…']);
    await user.click(within(popover).getByRole('button', { name: 'Privacy…' }));
    const settings = await screen.findByRole('dialog', { name: /settings/ });
    // Settings → Privacy follows the same rule (Q4-39): nothing there offers it either.
    expect(
      within(settings).queryByRole('button', { name: 'Make my connection public…' })
    ).toBeNull();
    expect(patches()).toEqual([]);
  });

  it('asks for the workspace name before the status-row popover makes the connection Public', async () => {
    // A workspace that allows Public: the one place the downgrade changes which models may read.
    savingDaemon({
      snapshot: richSnapshot({ workspace: { ...richSnapshot().workspace, mode: 'public' } }),
    });
    renderCrew();
    await channelReady();
    const user = userEvent.setup();

    await user.click(screen.getByRole('button', { name: /^Privacy: Private · ucsf/ }));
    await user.click(await screen.findByRole('button', { name: 'Make my connection public…' }));
    let dialog = await makePublicConfirmation();
    expect(within(dialog).getByRole('button', { name: 'Make public' })).toBeDisabled();
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(expectNothingOpen);
    expect(patches()).toEqual([]);

    await user.click(screen.getByRole('button', { name: /^Privacy: Private · ucsf/ }));
    await user.click(await screen.findByRole('button', { name: 'Make my connection public…' }));
    dialog = await makePublicConfirmation();
    await user.type(within(dialog).getByLabelText(`Type ${WORKSPACE} to confirm`), WORKSPACE);
    await user.click(within(dialog).getByRole('button', { name: 'Make public' }));

    await waitFor(() => expect(patches()).toEqual([fullBody('public')]));
  });

  it('asks the same from the Privacy tab of Workspace settings', async () => {
    // A workspace that allows Public: the only one where Settings offers the downgrade (Q4-39).
    savingDaemon({
      snapshot: richSnapshot({ workspace: { ...richSnapshot().workspace, mode: 'public' } }),
    });
    renderCrew();
    await channelReady();
    const user = userEvent.setup();

    await workspaceAction('Privacy…', /^lab/);
    const settings = await screen.findByRole('dialog', { name: /settings/ });
    await user.click(within(settings).getByRole('button', { name: 'Make my connection public…' }));
    let dialog = await makePublicConfirmation();
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    // Cancel steps back to the settings it came from, having changed nothing.
    await waitFor(() =>
      expect(
        screen.queryByRole('alertdialog', { name: `Make your ${WORKSPACE} connection public?` })
      ).toBeNull()
    );
    expect(patches()).toEqual([]);

    await user.click(
      within(await screen.findByRole('dialog', { name: /settings/ })).getByRole('button', {
        name: 'Make my connection public…',
      })
    );
    dialog = await makePublicConfirmation();
    await user.type(within(dialog).getByLabelText(`Type ${WORKSPACE} to confirm`), WORKSPACE);
    await user.click(within(dialog).getByRole('button', { name: 'Make public' }));

    await waitFor(() => expect(patches()).toEqual([fullBody('public')]));
  });

  it('asks the same when Connection settings is saved as Public, and sends the whole record', async () => {
    savingDaemon();
    renderCrew();
    await channelReady();
    const user = userEvent.setup();

    await workspaceAction('Connection settings…', /^lab/);
    await user.click(await screen.findByRole('radio', { name: /^Public/ }));
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    let dialog = await makePublicConfirmation();
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    expect(patches()).toEqual([]);

    await user.click(await screen.findByRole('button', { name: 'Save connection' }));
    dialog = await makePublicConfirmation();
    await user.type(within(dialog).getByLabelText(`Type ${WORKSPACE} to confirm`), WORKSPACE);
    await user.click(within(dialog).getByRole('button', { name: 'Make public' }));

    await waitFor(() => expect(patches()).toEqual([fullBody('public')]));
  });

  it('makes a Public connection Private in one click, with no confirmation', async () => {
    const publicConnection = { ...connection, mode: 'public' as const };
    savingDaemon({
      connections: [publicConnection],
      connectionMode: 'public',
      snapshot: richSnapshot({
        workspace: { ...richSnapshot().workspace, mode: 'public' },
      }),
    });
    renderCrew();
    await channelReady();
    const user = userEvent.setup();

    await user.click(screen.getByRole('button', { name: /^Privacy: Public/ }));
    await user.click(await screen.findByRole('button', { name: 'Make private' }));

    await waitFor(() => expect(patches()).toEqual([fullBody('private', publicConnection)]));
    expectNothingOpen();
  });

  it('confirms the permanent institution label before policy.set, from the note and the Privacy tab', async () => {
    installDaemon({
      snapshot: richSnapshot({
        workspace: { ...richSnapshot().workspace, institution_id: null },
      }),
    });
    renderCrew();
    await channelReady();
    const user = userEvent.setup();

    // The host's note above the composer.
    await user.click(await screen.findByRole('button', { name: 'Set institution to ucsf…' }));
    let dialog = await screen.findByRole('alertdialog', {
      name: `Set ${WORKSPACE}’s institution to ucsf?`,
    });
    expect(policySets()).toEqual([]);
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(expectNothingOpen);
    expect(policySets()).toEqual([]);

    // Workspace settings → Privacy.
    await workspaceAction('Privacy…', /^lab/);
    await user.click(
      within(await screen.findByRole('dialog', { name: /settings/ })).getByRole('button', {
        name: 'Set institution to ucsf…',
      })
    );
    dialog = await screen.findByRole('alertdialog', {
      name: `Set ${WORKSPACE}’s institution to ucsf?`,
    });
    expect(policySets()).toEqual([]);
    await user.click(within(dialog).getByRole('button', { name: 'Set ucsf permanently' }));

    await waitFor(() =>
      expect(policySets()).toEqual([{ mode: 'private', institution_id: 'ucsf' }])
    );
  });
});

/**
 * SF2-N7: Workspace settings stays open through the reconnect its own privacy change causes. The
 * daemon ends that view with `policy_changed` and `clear: true`, which drops the last verified copy
 * as well, so until the view verifies again the dialog has no snapshot. Drawn from none, the people
 * directory named nobody as the host, and the host who had just pressed Make private lost the
 * Workspace row and read a member's sentences on the Privacy and People tabs.
 */
describe('Workspace settings through the reconnect its privacy change causes (SF2-N7)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('stays open for the host, says Checking… meanwhile and never a member’s words, then draws the host’s view again', async () => {
    const publicConnection = { ...connection, mode: 'public' as const };
    const daemon = savingDaemon({
      connections: [publicConnection],
      connectionMode: 'public',
      snapshot: richSnapshot({ workspace: { ...richSnapshot().workspace, mode: 'public' } }),
    });
    renderCrew();
    await channelReady();
    const user = userEvent.setup();

    await workspaceAction('Privacy…', /^lab/);
    const settings = await screen.findByRole('dialog', { name: `${WORKSPACE} settings` });
    // The host's own control on the Workspace row, before.
    expect(
      within(settings).getByRole('button', { name: 'Make Private for everyone…' })
    ).toBeInTheDocument();

    // The daemon saves the connection as Private, then holds the next view back while it connects
    // again, and ends the view the refresh opened with the policy change (`clear: true`).
    daemon.state.connections = [{ ...publicConnection, mode: 'private' }];
    daemon.state.connectionMode = 'private';
    daemon.state.hold = true;
    const before = mocked.observeCrew.mock.calls.length;
    await user.click(within(settings).getByRole('button', { name: 'Make private' }));
    await waitFor(() => expect(patches()).toEqual([fullBody('private', publicConnection)]));
    await waitFor(() => expect(mocked.observeCrew.mock.calls.length).toBeGreaterThan(before));
    const ending = mocked.observeCrew.mock.calls.length;
    act(() =>
      daemon.emit({
        type: 'error',
        code: 'policy_changed',
        clear: true,
        error: 'Room observation ended.',
      })
    );

    // Nothing verified to draw: the dialog is still open, and says Checking… where the snapshot
    // decides, on every tab (they are all mounted), in no member's words.
    const checking = await screen.findByRole('dialog', { name: /settings$/ });
    for (const words of MEMBER_WORDS) expect(checking.textContent).not.toMatch(words);
    expect(checking.querySelector('.crew-settings-panels')).toHaveAttribute('aria-busy', 'true');
    expect(checking.querySelectorAll('[data-crew-settings-checking]').length).toBeGreaterThan(0);

    // It observes again by itself, still held: the same, and still open.
    await waitFor(() => expect(mocked.observeCrew.mock.calls.length).toBeGreaterThan(ending));
    const waiting = screen.getByRole('dialog', { name: /settings$/ });
    for (const words of MEMBER_WORDS) expect(waiting.textContent).not.toMatch(words);

    // The fresh view arrives: the host's view again, in the same dialog.
    act(() => daemon.release());
    const verified = await screen.findByRole('dialog', { name: `${WORKSPACE} settings` });
    await waitFor(() =>
      expect(
        within(verified).getByRole('button', { name: 'Make Private for everyone…' })
      ).toBeInTheDocument()
    );
    expect(verified.querySelector('[data-crew-settings-checking]')).toBeNull();
    expect(verified.querySelector('.crew-settings-panels')).not.toHaveAttribute('aria-busy');
    for (const words of MEMBER_WORDS) expect(verified.textContent).not.toMatch(words);
  });
});

/**
 * T3-UI-15: the real daemon's save of a connected connection (`CrewManager::update`) disconnects,
 * saves and connects again, all inside the one PATCH. The saved record reads `disconnected` from
 * the moment the PATCH arrives until the reconnect is done, and the old observer, waking from its
 * 2 s idle, ends with `policy_changed` while the PATCH is still on its way. An observation opened
 * meanwhile meets no transport and ends `observation_refused`. Taking those ends for a dropped
 * connection closed Workspace settings and flashed an error until the PATCH came back.
 */
describe('an end while this window’s own save of the connection is on its way (T3-UI-15)', () => {
  const DAEMON_SENTENCE = 'Room observation ended.';
  const OBSERVATION_REFUSED = {
    type: 'error',
    code: 'observation_refused',
    clear: true,
    error: DAEMON_SENTENCE,
  };
  /** Statuses an end decided as a loss (or a failure) would have drawn. */
  const DECIDED = ['offline', 'updates-unavailable', 'cant-connect', 'reconnecting'];

  /** Remembers whether an alert ever appeared in the connection bar, however briefly. */
  function watchTheBar() {
    const seen = { alert: false };
    const check = () => {
      const bar = document.querySelector('[data-testid="crew-connection-bar"]');
      if (bar?.querySelector('[role="alert"]')) seen.alert = true;
    };
    const observer = new MutationObserver(check);
    observer.observe(document.body, { childList: true, subtree: true, characterData: true });
    check();
    return { seen, stop: () => observer.disconnect() };
  }
  let watcher: ReturnType<typeof watchTheBar> | null = null;

  /**
   * Hold every PATCH of the connection as the real daemon's reconnect would, until `answer()` (the
   * reconnect worked) or `fail(failure)` (the save failed after the bridge was dropped, which leaves
   * the record `disconnected`). Any observation while the record is not connected ends
   * `observation_refused`, as one that meets no transport does. With `reconnects: false` the save
   * connects nothing, as the daemon's save of a connection with no bridge does.
   */
  function reconnectOnSave(daemon: ScriptedDaemon, { reconnects = true } = {}) {
    let settle: ((failure?: unknown) => void) | null = null;
    daemon.state.http = (path, method, body) => {
      if (path !== `/connections/${connection.id}` || method !== 'PATCH') return undefined;
      const record = { ...(daemon.state.connections[0] as CrewConnection), ...(body as object) };
      daemon.state.connections = [{ ...record, status: 'disconnected' }];
      return new Promise((resolve, reject) => {
        settle = (failure) => {
          settle = null;
          if (failure !== undefined) {
            reject(failure);
            return;
          }
          const status = reconnects ? 'connected' : 'disconnected';
          daemon.state.connections = [{ ...record, status }];
          daemon.state.connectionMode = record.mode;
          resolve({ ...record, status });
        };
      });
    };
    const observe = mocked.observeCrew.getMockImplementation();
    if (!observe) throw new Error('installDaemon sets the observer first.');
    mocked.observeCrew.mockImplementation(
      async (
        connectionId: string,
        channelId: string | undefined,
        after: string | null,
        signal: AbortSignal,
        deliver: (frame: unknown) => void
      ) => {
        const saved = daemon.state.connections.find((item) => item.id === connectionId);
        if (!signal.aborted && saved?.status !== 'connected') {
          deliver(OBSERVATION_REFUSED);
          return 'terminal';
        }
        return observe(connectionId, channelId, after, signal, deliver);
      }
    );
    return {
      saving: () => settle !== null,
      answer: () => settle?.(),
      fail: (failure: unknown) => settle?.(failure),
    };
  }

  /** Long enough for the first automatic re-observation, and a loss decided after it. */
  async function waitPastTheFirstReobservation() {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, REOBSERVE_BACKOFF_MS[0] + 700));
    });
  }

  /** The old observer wakes and finds the binding it was opened under has moved. */
  function policyChanged(daemon: ScriptedDaemon) {
    act(() =>
      daemon.emit({ type: 'error', code: 'policy_changed', clear: true, error: DAEMON_SENTENCE })
    );
  }

  beforeEach(() => {
    vi.clearAllMocks();
  });
  afterEach(() => {
    watcher?.stop();
    watcher = null;
  });

  it('keeps Workspace settings open saying Checking…, with no error, then draws the host’s view once the PATCH is back', async () => {
    const publicConnection = { ...connection, mode: 'public' as const };
    const daemon = installDaemon({
      connections: [publicConnection],
      connectionMode: 'public',
      snapshot: richSnapshot({ workspace: { ...richSnapshot().workspace, mode: 'public' } }),
    });
    const save = reconnectOnSave(daemon);
    renderCrew();
    await channelReady();
    const user = userEvent.setup();

    await workspaceAction('Privacy…', /^lab/);
    const settings = await screen.findByRole('dialog', { name: `${WORKSPACE} settings` });
    watcher = watchTheBar();
    await user.click(within(settings).getByRole('button', { name: 'Make private' }));
    await waitFor(() => expect(save.saving()).toBe(true));
    expect(patches()).toEqual([fullBody('private', publicConnection)]);

    // The old observer wakes inside the PATCH and ends the view: the renderer's list still says
    // connected, and the daemon's says disconnected.
    policyChanged(daemon);
    await waitPastTheFirstReobservation();

    // Still the save's: open, Checking…, no member's words, no error, nothing decided.
    const checking = screen.getByRole('dialog', { name: /settings$/ });
    expect(checking.querySelector('.crew-settings-panels')).toHaveAttribute('aria-busy', 'true');
    expect(checking.querySelectorAll('[data-crew-settings-checking]').length).toBeGreaterThan(0);
    for (const words of MEMBER_WORDS) expect(checking.textContent).not.toMatch(words);
    expect(currentCrew().refreshError).toBeNull();
    expect(currentCrew().error).toBeNull();
    expect(watcher.seen.alert).toBe(false);
    expect(renderedStatuses().filter((status) => DECIDED.includes(status ?? ''))).toEqual([]);

    // The reconnect is done and the PATCH answers: the save's own refresh verifies the view, and
    // is the one observation that follows it.
    const beforeAnswer = mocked.observeCrew.mock.calls.length;
    await act(async () => save.answer());
    const verified = await screen.findByRole('dialog', { name: `${WORKSPACE} settings` });
    await waitFor(() =>
      expect(
        within(verified).getByRole('button', { name: 'Make Private for everyone…' })
      ).toBeInTheDocument()
    );
    expect(verified.querySelector('[data-crew-settings-checking]')).toBeNull();
    for (const words of MEMBER_WORDS) expect(verified.textContent).not.toMatch(words);
    expect(mocked.observeCrew.mock.calls.length).toBe(beforeAnswer + 1);
    expect(currentCrew().status).toBe('connected');
    expect(currentCrew().refreshError).toBeNull();
    expect(watcher.seen.alert).toBe(false);
    expect(renderedStatuses().filter((status) => DECIDED.includes(status ?? ''))).toEqual([]);
  });

  it('observes again by itself once a save that refreshes nothing is back (a rename)', async () => {
    const daemon = installDaemon();
    const save = reconnectOnSave(daemon);
    renderCrew();
    await channelReady();
    watcher = watchTheBar();

    let saved: Promise<unknown> = Promise.resolve();
    act(() => {
      saved = currentCrew().updateConnection(connection.id, {
        ...connectionUpdateBody(connection),
        name: 'Renamed',
      });
    });
    await waitFor(() => expect(save.saving()).toBe(true));
    const before = mocked.observeCrew.mock.calls.length;
    policyChanged(daemon);
    await waitPastTheFirstReobservation();

    // Nothing is observed, and nothing decided, while the save is on its way.
    expect(mocked.observeCrew.mock.calls.length).toBe(before);
    expect(currentCrew().refreshError).toBeNull();
    expect(currentCrew().status).toBe('updating');

    await act(async () => {
      save.answer();
      await saved;
    });
    // No caller refreshed: the end left to the save is observed again once it is back.
    await channelReady();
    expect(mocked.observeCrew.mock.calls.length).toBeGreaterThan(before);
    expect(currentCrew().status).toBe('connected');
    expect(currentCrew().refreshError).toBeNull();
    expect(watcher.seen.alert).toBe(false);
    expect(renderedStatuses().filter((status) => DECIDED.includes(status ?? ''))).toEqual([]);
  });

  it('leaves a loss decision whose record read lands inside a save to that save', async () => {
    const daemon = installDaemon();
    const save = reconnectOnSave(daemon);
    // The loss handler's read of the saved record is held until the save has begun.
    const onSave = daemon.state.http;
    let holdList = false;
    let releaseList: (() => void) | null = null;
    daemon.state.http = (path, method, body) => {
      if (holdList && path === '/connections' && method === 'GET') {
        holdList = false;
        return new Promise((resolve) => {
          releaseList = () => resolve({ connections: daemon.state.connections });
        });
      }
      return onSave?.(path, method, body);
    };
    renderCrew();
    await channelReady();
    watcher = watchTheBar();

    // An end a dropped connection explains, while the daemon still calls it connected: the loss
    // handler reads the record again, and that read is slow.
    holdList = true;
    act(() => daemon.emit(OBSERVATION_REFUSED));
    await waitFor(() => expect(releaseList).not.toBeNull());
    let saved: Promise<unknown> = Promise.resolve();
    act(() => {
      saved = currentCrew().updateConnection(connection.id, {
        ...connectionUpdateBody(connection),
        name: 'Renamed',
      });
    });
    await waitFor(() => expect(save.saving()).toBe(true));
    // The read answers inside the save, where the record says disconnected: not a loss to decide.
    await act(async () => releaseList?.());
    await waitPastTheFirstReobservation();
    expect(currentCrew().refreshError).toBeNull();
    expect(currentCrew().status).toBe('updating');
    expect(watcher.seen.alert).toBe(false);

    await act(async () => {
      save.answer();
      await saved;
    });
    await channelReady();
    expect(currentCrew().status).toBe('connected');
    expect(currentCrew().refreshError).toBeNull();
    expect(watcher.seen.alert).toBe(false);
    expect(
      renderedStatuses().filter((status) =>
        ['offline', 'updates-unavailable', 'cant-connect'].includes(status ?? '')
      )
    ).toEqual([]);
  });

  it('leaves nothing to a save of a connection that was not connected: it reads offline throughout', async () => {
    const daemon = installDaemon();
    const save = reconnectOnSave(daemon, { reconnects: false });
    renderCrew();
    await channelReady();
    // The bridge dropped and the daemon says so: offline, decided as ever.
    daemon.state.connections = [{ ...connection, status: 'disconnected' }];
    act(() => daemon.emit(OBSERVATION_REFUSED));
    await waitFor(() => expect(currentCrew().status).toBe('offline'));

    // Its settings are saved while it is offline: the daemon reconnects nothing inside that save.
    let saved: Promise<unknown> = Promise.resolve();
    act(() => {
      saved = currentCrew().updateConnection(connection.id, {
        ...connectionUpdateBody(connection),
        name: 'Renamed',
      });
    });
    await waitFor(() => expect(save.saving()).toBe(true));
    const since = renderedStatuses().length;
    await act(async () => {
      await currentCrew().refresh();
    });
    await waitFor(() => expect(currentCrew().refreshError).not.toBeNull());
    expect(currentCrew().status).toBe('offline');
    expect(currentCrew().screen).toBe('offline');
    expect(renderedStatuses().slice(since)).not.toContain('updating');

    await act(async () => {
      save.answer();
      await saved;
    });
    expect(currentCrew().status).toBe('offline');
  });

  it('decides by the record once a save fails after the bridge dropped: offline, never left checking', async () => {
    const daemon = installDaemon();
    const save = reconnectOnSave(daemon);
    renderCrew();
    await channelReady();

    let saved: Promise<unknown> = Promise.resolve();
    act(() => {
      saved = currentCrew()
        .updateConnection(connection.id, { ...connectionUpdateBody(connection), name: 'Renamed' })
        .catch(() => undefined);
    });
    await waitFor(() => expect(save.saving()).toBe(true));
    policyChanged(daemon);
    await act(async () => {
      save.fail(new CrewHttpError('The connection could not be saved.', 500, 'internal'));
      await saved;
    });

    await waitFor(() => expect(currentCrew().status).toBe('offline'));
    expect(currentCrew().screen).toBe('offline');
  });

  it('still shows the workspace’s own answer at once while a save is on its way', async () => {
    const publicConnection = { ...connection, mode: 'public' as const };
    const daemon = installDaemon({
      connections: [publicConnection],
      connectionMode: 'public',
      snapshot: richSnapshot({ workspace: { ...richSnapshot().workspace, mode: 'public' } }),
    });
    const save = reconnectOnSave(daemon);
    renderCrew();
    await channelReady();
    const user = userEvent.setup();

    await workspaceAction('Privacy…', /^lab/);
    const settings = await screen.findByRole('dialog', { name: `${WORKSPACE} settings` });
    await user.click(within(settings).getByRole('button', { name: 'Make private' }));
    await waitFor(() => expect(save.saving()).toBe(true));

    // Removed from the workspace: an answer about the person, never the save's doing.
    act(() =>
      daemon.emit({ type: 'error', code: 'principal_revoked', clear: true, error: DAEMON_SENTENCE })
    );
    await waitFor(() =>
      expect(currentCrew().refreshError).toBe(crewObservationCopy.noLongerMember(WORKSPACE))
    );
    expect(screen.queryByRole('dialog', { name: /settings$/ })).toBeNull();
    await act(async () => save.answer());
  });
});
