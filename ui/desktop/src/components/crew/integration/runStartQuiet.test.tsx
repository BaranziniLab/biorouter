import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeAll, afterAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { composerCopy } from '../composer/copy';
import { crewStatusCopy } from '../state/copy';
import { RUN_START_FRAME_WAIT_MS } from '../state/crewRunStart';
import { chooseModel, installResizeObserverStub } from '../test/crewTestUtils';
import {
  channelReady,
  connection,
  currentCrew,
  ids,
  installDaemon,
  mocked,
  ownedRun,
  renderCrew,
  renderedStatuses,
  richMessages,
  type ScriptedDaemon,
} from './harness';

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
// Stable across renders, as the real context's callbacks are.
const config = vi.hoisted(() => ({
  getProviders: async () => [{ name: 'fixture-provider', is_configured: true }],
  read: async () => '',
  getProviderModels: async () => ['fixture-model'],
}));
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return { ...actual, useConfig: () => config };
});
vi.mock('../CrewAuthentication', () => ({ default: () => <div /> }));

installResizeObserverStub();

// jsdom has no layout, so it has no `scrollIntoView`; the timeline brings a new task row into view.
const original = Element.prototype.scrollIntoView;
beforeAll(() => {
  Element.prototype.scrollIntoView = vi.fn();
});
afterAll(() => {
  Element.prototype.scrollIntoView = original;
});

/**
 * Live QA round 3, Q3-06 (SECURITY-SENSITIVE: human review). Pressing Start in Ask my agent used
 * to end with a refresh: the verified view was dropped and the channel read "Checking connection"
 * and "Verifying access…" for 3–5 s, right after the person's own action. A successful start now
 * keeps the verified view, the messages and the composer, and the observer's next verified `state`
 * frame brings the task. Every observer refusal after it still clears what the view protected.
 */

const RUNS = `/connections/${connection.id}/runs`;

/** Remembers whether the page ever said it was checking or verifying, however briefly. */
function watchForVerifying() {
  const seen = { verifying: false, checking: false };
  const check = () => {
    const text = document.body.textContent ?? '';
    if (text.includes(composerCopy.verifying)) seen.verifying = true;
    if (text.includes(crewStatusCopy.checking)) seen.checking = true;
  };
  const observer = new MutationObserver(check);
  observer.observe(document.body, { childList: true, subtree: true, characterData: true });
  check();
  return { seen, stop: () => observer.disconnect() };
}

function statusRow(): HTMLElement {
  return within(screen.getByRole('navigation', { name: 'Crew' })).getByRole('status', {
    name: /connection status/i,
  });
}

function taskRow(): HTMLElement | null {
  return document.querySelector<HTMLElement>('.crew-task-row');
}

function starts(): number {
  return mocked.crewHttp.mock.calls.filter(([path, method]) => path === RUNS && method === 'POST')
    .length;
}

let daemon: ScriptedDaemon;
let watcher: ReturnType<typeof watchForVerifying> | null = null;

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  daemon = installDaemon({
    // The channel's history without the task's own post: the task row is the one thing to arrive.
    messages: richMessages().filter((message) => !message.run_id),
    stateAfterStart: false,
    http: (path, method) => {
      if (path === RUNS && method === 'POST') {
        // The daemon admitted the task: its observer's next state frame lists it.
        daemon.state.runs = [ownedRun];
        return { ...ownedRun, status: 'running' };
      }
      return undefined;
    },
  });
});
afterEach(() => {
  watcher?.stop();
  watcher = null;
  vi.useRealTimers();
});

/**
 * Start the viewer's agent and leave the start unanswered, as a daemon still creating the provider
 * and running its preflight does for seconds. `answer` admits the task; `started` is the Start's
 * own result.
 */
function startHeldTask() {
  let admit: (value: unknown) => void = () => undefined;
  daemon.state.http = (path, method) => {
    if (path === RUNS && method === 'POST')
      return new Promise((resolve) => {
        admit = resolve;
      });
    if (path === `/connections/${connection.id}/disconnect` && method === 'POST') {
      daemon.state.connections = [{ ...connection, status: 'disconnected' }];
      return {};
    }
    return undefined;
  };
  let started: Promise<boolean> = Promise.resolve(false);
  act(() => {
    started = currentCrew().startOwnedRun({
      prompt: 'plot counts by sample',
      provider: 'fixture-provider',
      model: 'fixture-model',
      contextChannels: [],
    });
  });
  return {
    async answer() {
      await act(async () => {
        admit({ ...ownedRun, status: 'running' });
        expect(await started).toBe(true);
      });
    },
  };
}

/** Start the viewer's agent through the controller, as the pane's Start does. */
async function startTask() {
  let started: boolean | undefined;
  await act(async () => {
    started = await currentCrew().startOwnedRun({
      prompt: 'plot counts by sample',
      provider: 'fixture-provider',
      model: 'fixture-model',
      contextChannels: [],
    });
  });
  expect(started).toBe(true);
}

describe('a successful Start never blanks the channel (Q3-06)', () => {
  it('stays Connected with the messages and composer on screen, and the task arrives with the next state frame', async () => {
    renderCrew();
    await channelReady();
    expect(taskRow()).toBeNull();
    watcher = watchForVerifying();
    const observations = mocked.observeCrew.mock.calls.length;
    const rendered = renderedStatuses().length;

    // Through the pane, as the person does it.
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    fireEvent.change(await screen.findByLabelText('Task'), {
      target: { value: 'plot counts by sample' },
    });
    await chooseModel('fixture-model');
    await user.click(screen.getByRole('button', { name: 'Start my agent and allow posting here' }));
    await waitFor(() => expect(starts()).toBe(1));
    // Let everything the start set off settle.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 50));
    });

    // Nothing was dropped or observed again: the view the person pressed Start on is still there.
    expect(mocked.observeCrew.mock.calls.length).toBe(observations);
    expect(screen.getByRole('textbox', { name: 'Message #general' })).toBeInTheDocument();
    expect(screen.getByText('Plot next?')).toBeInTheDocument();
    expect(within(statusRow()).getByText(crewStatusCopy.connected)).toBeInTheDocument();
    expect(taskRow()).toBeNull();

    // The observer's next verified state frame lists the task.
    act(() => daemon.emitState());
    await waitFor(() => expect(taskRow()).not.toBeNull());

    expect(watcher.seen.verifying).toBe(false);
    expect(watcher.seen.checking).toBe(false);
    expect(renderedStatuses().slice(rendered)).not.toHaveLength(0);
    expect(new Set(renderedStatuses().slice(rendered))).toEqual(new Set(['connected']));
    expect(mocked.observeCrew.mock.calls.length).toBe(observations);
  });

  it('observes again, keeping the view on screen, when no frame lists the task in time', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    renderCrew();
    await channelReady();
    watcher = watchForVerifying();
    const observations = mocked.observeCrew.mock.calls.length;
    const rendered = renderedStatuses().length;

    await startTask();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(RUN_START_FRAME_WAIT_MS - 500);
    });
    expect(mocked.observeCrew.mock.calls.length).toBe(observations);
    expect(taskRow()).toBeNull();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    // Observed again once, without a refresh: the new observation's first frame brings the task.
    await waitFor(() => expect(taskRow()).not.toBeNull());
    expect(mocked.observeCrew.mock.calls.length).toBe(observations + 1);
    expect(screen.getByRole('textbox', { name: 'Message #general' })).toBeInTheDocument();
    expect(watcher.seen.verifying).toBe(false);
    expect(new Set(renderedStatuses().slice(rendered))).toEqual(new Set(['connected']));

    // Once is enough: nothing observes again after the task was listed.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(RUN_START_FRAME_WAIT_MS * 3);
    });
    expect(mocked.observeCrew.mock.calls.length).toBe(observations + 1);
  });

  it('does not wait when a frame listed the task before the start was answered', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    renderCrew();
    await channelReady();
    daemon.state.http = (path, method) => {
      if (path === RUNS && method === 'POST') {
        daemon.state.runs = [ownedRun];
        // The observer's periodic frame lands while the start is still being answered.
        daemon.emitState();
        return { ...ownedRun, status: 'running' };
      }
      return undefined;
    };
    const observations = mocked.observeCrew.mock.calls.length;
    await startTask();
    await waitFor(() => expect(taskRow()).not.toBeNull());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(RUN_START_FRAME_WAIT_MS * 2);
    });
    expect(mocked.observeCrew.mock.calls.length).toBe(observations);
  });

  it.each([['access_denied'], ['privacy_denied'], ['principal_revoked']])(
    'still clears everything when the observer ends with %s after a Start, and never observes again by itself',
    async (code) => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      renderCrew();
      await channelReady();
      await startTask();
      const observations = mocked.observeCrew.mock.calls.length;

      act(() =>
        daemon.emit({ type: 'error', code, clear: true, error: 'Room observation ended.' })
      );

      await waitFor(() => expect(currentCrew().snapshot).toBeNull());
      expect(currentCrew().observedPrivacy).toBeNull();
      expect(currentCrew().messages).toEqual([]);
      expect(currentCrew().runs).toEqual([]);
      expect(currentCrew().refreshError).not.toBeNull();
      expect(currentCrew().status).not.toBe('connected');
      expect(screen.queryByRole('textbox', { name: 'Message #general' })).toBeNull();
      expect(screen.queryByText('Plot next?')).toBeNull();

      // The start's wait ended with the view: it never brings a refused view back by itself.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(RUN_START_FRAME_WAIT_MS * 3);
      });
      expect(mocked.observeCrew.mock.calls.length).toBe(observations);
      expect(currentCrew().snapshot).toBeNull();
    }
  );

  // Review of Q3-06: the wait used to be armed however the view stood when the start was
  // answered, and only an end that came after it cancelled the wait. An end while the start was
  // still being answered left it armed, and 5 s later it brought the refused view back by itself.
  it.each([['access_denied'], ['privacy_denied'], ['principal_revoked']])(
    'never observes again by itself when the observer ends with %s while the start is still being answered',
    async (code) => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      renderCrew();
      await channelReady();
      const start = startHeldTask();
      await waitFor(() => expect(starts()).toBe(1));

      act(() =>
        daemon.emit({ type: 'error', code, clear: true, error: 'Room observation ended.' })
      );
      await waitFor(() => expect(currentCrew().snapshot).toBeNull());
      const observations = mocked.observeCrew.mock.calls.length;

      // The daemon admits the task after the view ended.
      await start.answer();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(RUN_START_FRAME_WAIT_MS * 3);
      });

      expect(mocked.observeCrew.mock.calls.length).toBe(observations);
      expect(currentCrew().snapshot).toBeNull();
      expect(currentCrew().observedPrivacy).toBeNull();
      expect(currentCrew().refreshError).not.toBeNull();
      expect(currentCrew().status).not.toBe('connected');
      expect(screen.queryByRole('textbox', { name: 'Message #general' })).toBeNull();
    }
  );

  it('never observes a connection the person disconnected while the start was being answered', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    renderCrew();
    await channelReady();
    const start = startHeldTask();
    await waitFor(() => expect(starts()).toBe(1));

    await act(async () => {
      await currentCrew().disconnect();
    });
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));
    const observations = mocked.observeCrew.mock.calls.length;

    await start.answer();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(RUN_START_FRAME_WAIT_MS * 3);
    });

    expect(mocked.observeCrew.mock.calls.length).toBe(observations);
    expect(currentCrew().snapshot).toBeNull();
    expect(currentCrew().screen).toBe('offline');
  });

  it('never starts the newly selected connection over when the selection moved while the start was being answered', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const other = { ...connection, id: 'conn-2', name: 'Second fixture' };
    daemon.state.connections = [connection, other];
    renderCrew();
    await channelReady();
    const start = startHeldTask();
    await waitFor(() => expect(starts()).toBe(1));

    act(() => currentCrew().selectConnection(other.id));
    await waitFor(() => expect(currentCrew().observedPrivacy?.connectionId).toBe(other.id));
    await channelReady();
    const observations = mocked.observeCrew.mock.calls.length;

    // The start was for the first connection; the second one's view is not this wait's.
    await start.answer();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(RUN_START_FRAME_WAIT_MS * 3);
    });

    expect(mocked.observeCrew.mock.calls.length).toBe(observations);
    expect(currentCrew().observedPrivacy?.connectionId).toBe(other.id);
  });

  it('leaves a channel selected during the wait to its own observation, which is never started over by the wait', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    renderCrew();
    await channelReady();
    const start = startHeldTask();
    await start.answer();

    // The view stays verified and on the same connection: only the observation behind it moved.
    act(() => currentCrew().selectChannel(ids.methods));
    await channelReady('methods');
    const observations = mocked.observeCrew.mock.calls.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(RUN_START_FRAME_WAIT_MS * 3);
    });
    expect(mocked.observeCrew.mock.calls.length).toBe(observations);
    expect(currentCrew().status).toBe('connected');
  });

  it('still waits, and observes again once, when the view stayed verified while the start was being answered', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    renderCrew();
    await channelReady();
    const start = startHeldTask();
    await waitFor(() => expect(starts()).toBe(1));
    // The observer's periodic frame, without the task, lands while the start is being answered.
    act(() => daemon.emitState());
    const observations = mocked.observeCrew.mock.calls.length;

    await start.answer();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(RUN_START_FRAME_WAIT_MS - 500);
    });
    expect(mocked.observeCrew.mock.calls.length).toBe(observations);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(RUN_START_FRAME_WAIT_MS * 3);
    });
    expect(mocked.observeCrew.mock.calls.length).toBe(observations + 1);
    expect(currentCrew().status).toBe('connected');
  });

  it('keeps the old behaviour for a refused start: nothing is waited for or observed again', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    daemon.state.http = (path, method) => {
      if (path === RUNS && method === 'POST') throw new Error('start failed');
      return undefined;
    };
    renderCrew();
    await channelReady();
    const observations = mocked.observeCrew.mock.calls.length;
    let started: boolean | undefined;
    await act(async () => {
      started = await currentCrew().startOwnedRun({
        prompt: 'plot counts by sample',
        provider: 'fixture-provider',
        model: 'fixture-model',
        contextChannels: [],
      });
    });
    expect(started).toBe(false);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(RUN_START_FRAME_WAIT_MS * 2);
    });
    expect(mocked.observeCrew.mock.calls.length).toBe(observations);
    expect(currentCrew().status).toBe('connected');
  });
});
