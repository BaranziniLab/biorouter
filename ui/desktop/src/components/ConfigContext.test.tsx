import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ConfigProvider, useConfig } from './ConfigContext';

// Issue #52 — the cached `config` object was only ever re-read when a write
// went through this context's own `upsert`. Every key written straight to the
// API (`setConfigProvider`, notably) left the cache serving whatever was loaded
// when the provider mounted, with no way for a consumer to ask for a re-read:
// the reload function existed but was private.

const mocks = vi.hoisted(() => ({
  readAllConfig: vi.fn(),
  readConfig: vi.fn(),
  removeConfig: vi.fn(),
  upsertConfig: vi.fn(),
  getExtensions: vi.fn(),
  addExtension: vi.fn(),
  removeExtension: vi.fn(),
  providers: vi.fn(),
  getProviderModels: vi.fn(),
  syncBundledExtensions: vi.fn(),
  catalogChanges: vi.fn(),
}));

vi.mock('../api', () => ({
  readAllConfig: mocks.readAllConfig,
  readConfig: mocks.readConfig,
  removeConfig: mocks.removeConfig,
  upsertConfig: mocks.upsertConfig,
  getExtensions: mocks.getExtensions,
  addExtension: mocks.addExtension,
  removeExtension: mocks.removeExtension,
  providers: mocks.providers,
  getProviderModels: mocks.getProviderModels,
  catalogChanges: mocks.catalogChanges,
}));

vi.mock('./settings/extensions', () => ({
  syncBundledExtensions: mocks.syncBundledExtensions,
}));

function ConfigProbe() {
  const { config, refreshConfig, upsert } = useConfig();
  const [refreshError, setRefreshError] = useState('none');
  const [upsertResult, setUpsertResult] = useState('idle');

  return (
    <div>
      <div data-testid="provider">{String(config.BIOROUTER_PROVIDER ?? 'none')}</div>
      <div data-testid="refresh-error">{refreshError}</div>
      <div data-testid="upsert-result">{upsertResult}</div>
      <button
        type="button"
        onClick={() => {
          setRefreshError('none');
          refreshConfig().catch((error: unknown) =>
            setRefreshError(error instanceof Error ? error.message : String(error))
          );
        }}
      >
        Refresh
      </button>
      <button
        type="button"
        onClick={() => {
          setUpsertResult('pending');
          upsert('BIOROUTER_MODE', 'smart_approve', false).then(
            () => setUpsertResult('ok'),
            (error: unknown) =>
              setUpsertResult(`failed: ${error instanceof Error ? error.message : String(error)}`)
          );
        }}
      >
        Upsert
      </button>
    </div>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  mocks.readAllConfig.mockResolvedValue({ data: { config: { BIOROUTER_PROVIDER: 'ollama' } } });
  mocks.getExtensions.mockResolvedValue({
    data: { extensions: [], warnings: [] },
    response: { status: 200 },
  });
  mocks.providers.mockResolvedValue({ data: [] });
  mocks.syncBundledExtensions.mockResolvedValue(undefined);
  mocks.upsertConfig.mockResolvedValue({ data: {} });
  // The quiet daemon: park and never answer. Any test that wants the catalogue
  // to move says so itself.
  mocks.catalogChanges.mockImplementation(() => new Promise(() => {}));
});

function renderProbe() {
  return render(
    <ConfigProvider>
      <ConfigProbe />
    </ConfigProvider>
  );
}

describe('ConfigContext refreshConfig (#52)', () => {
  it('re-reads the backing config on demand', async () => {
    render(
      <ConfigProvider>
        <ConfigProbe />
      </ConfigProvider>
    );

    await waitFor(() => expect(screen.getByTestId('provider')).toHaveTextContent('ollama'));

    // Someone else wrote the config — a model switch through `setConfigProvider`,
    // the CLI, another window. Nothing went through `upsert`, so the cache
    // cannot know.
    mocks.readAllConfig.mockResolvedValue({
      data: { config: { BIOROUTER_PROVIDER: 'versa_azure' } },
    });

    screen.getByRole('button', { name: 'Refresh' }).click();

    await waitFor(() => expect(screen.getByTestId('provider')).toHaveTextContent('versa_azure'));
  });
});

// The re-read added for #52 is the only thing that keeps the cache honest, so
// every way it can go wrong has to leave the cache no worse than it found it.
// The two ways it went wrong: a failed read replaced the whole cache with `{}`
// (the generated client does not throw by default, so an HTTP 500 *resolves*
// with no `data`), and two overlapping reads could apply in either order, so a
// read issued before a write could land after the refresh that followed it.
describe('ConfigContext cache integrity', () => {
  it('reads with throwOnError, so a failed read cannot arrive as an empty config', async () => {
    renderProbe();

    await waitFor(() => expect(screen.getByTestId('provider')).toHaveTextContent('ollama'));

    expect(mocks.readAllConfig).toHaveBeenCalledWith({ throwOnError: true });
    expect(mocks.readAllConfig).not.toHaveBeenCalledWith();
  });

  it('keeps the cached config when a re-read resolves without data', async () => {
    renderProbe();
    await waitFor(() => expect(screen.getByTestId('provider')).toHaveTextContent('ollama'));

    // What an HTTP 500 looks like through the non-throwing client: a resolved
    // response carrying an error and no config at all.
    mocks.readAllConfig.mockResolvedValue({
      data: undefined,
      error: { message: 'internal server error' },
      response: { status: 500 },
    });

    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));

    await waitFor(() => expect(mocks.readAllConfig).toHaveBeenCalledTimes(2));
    await waitFor(() =>
      expect(screen.getByTestId('refresh-error')).toHaveTextContent(/no configuration/i)
    );
    expect(screen.getByTestId('provider')).toHaveTextContent('ollama');
  });

  it('keeps the cached config when a re-read rejects, and reports the failure', async () => {
    renderProbe();
    await waitFor(() => expect(screen.getByTestId('provider')).toHaveTextContent('ollama'));

    mocks.readAllConfig.mockRejectedValue(new Error('daemon unreachable'));

    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));

    await waitFor(() =>
      expect(screen.getByTestId('refresh-error')).toHaveTextContent('daemon unreachable')
    );
    expect(screen.getByTestId('provider')).toHaveTextContent('ollama');
  });

  it('ignores a read that was issued earlier but landed later', async () => {
    let releaseMountRead: () => void = () => {};
    const mountRead = new Promise<void>((resolve) => {
      releaseMountRead = resolve;
    });

    mocks.readAllConfig
      .mockImplementationOnce(async () => {
        await mountRead;
        return { data: { config: { BIOROUTER_PROVIDER: 'ollama' } } };
      })
      .mockImplementation(async () => ({
        data: { config: { BIOROUTER_PROVIDER: 'versa_azure' } },
      }));

    renderProbe();

    // The mount read is still in flight when a provider write lands and asks
    // for a refresh. The refresh answers first.
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(screen.getByTestId('provider')).toHaveTextContent('versa_azure'));

    // Now the older read finally answers — with the pre-write snapshot.
    await act(async () => {
      releaseMountRead();
    });

    expect(screen.getByTestId('provider')).toHaveTextContent('versa_azure');
  });

  it('still applies an in-flight read when a newer read fails', async () => {
    let releaseMountRead: () => void = () => {};
    const mountRead = new Promise<void>((resolve) => {
      releaseMountRead = resolve;
    });

    mocks.readAllConfig
      .mockImplementationOnce(async () => {
        await mountRead;
        return { data: { config: { BIOROUTER_PROVIDER: 'ollama' } } };
      })
      .mockRejectedValue(new Error('daemon unreachable'));

    renderProbe();

    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() =>
      expect(screen.getByTestId('refresh-error')).toHaveTextContent('daemon unreachable')
    );

    // The refresh failed, so it never became the cache's contents. Discarding
    // the older read on its account would leave the cache empty for good.
    await act(async () => {
      releaseMountRead();
    });

    expect(screen.getByTestId('provider')).toHaveTextContent('ollama');
  });

  it('still loads providers and extensions when the initial config read fails', async () => {
    mocks.readAllConfig.mockRejectedValue(new Error('daemon still starting'));

    renderProbe();

    await waitFor(() => expect(mocks.providers).toHaveBeenCalled());
    await waitFor(() => expect(mocks.getExtensions).toHaveBeenCalled());
    expect(screen.getByTestId('provider')).toHaveTextContent('none');
  });

  it('does not fail a write because the re-read that follows it failed', async () => {
    renderProbe();
    await waitFor(() => expect(screen.getByTestId('provider')).toHaveTextContent('ollama'));

    mocks.readAllConfig.mockRejectedValue(new Error('daemon unreachable'));

    fireEvent.click(screen.getByRole('button', { name: 'Upsert' }));

    // The write landed. Reporting it as failed because a *cache* could not be
    // re-read would be wrong in the more alarming direction, and callers that
    // write several keys in sequence would stop partway through.
    await waitFor(() => expect(screen.getByTestId('upsert-result')).toHaveTextContent('ok'));
    expect(mocks.upsertConfig).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('provider')).toHaveTextContent('ollama');
  });
});

/**
 * Issue #112's catalogue subscription, against a daemon that has something to
 * report — which is every daemon whose revision is not zero.
 *
 * `GET /catalog/changes?since=0` does NOT park: a caller at revision 0 is
 * already behind, so the daemon answers immediately with the current revision
 * and the buffered changes. That is correct, and it is the whole hazard: a
 * subscription that is torn down and restarted resets its cursor to 0, so the
 * restart is answered instantly and the handler fires again. If the handler's
 * own work changes the identity of anything the subscribing effect depends on,
 * the effect re-subscribes and the two feed each other with a loopback round
 * trip (~1 ms) as the only brake.
 *
 * That is not a slow leak. Chromium allows six sockets per host, `stop()` does
 * not abort the request already on the wire, and each turn of the loop opens
 * another — so within a few hundred milliseconds every socket to the daemon is
 * spoken for and *every other request the renderer makes queues behind them and
 * never runs*: the model picker's bind, the provider model lists, the
 * diagnostics bundle, the chat reply. The window looks alive and answers
 * nothing, which is exactly how it was reported ("the Select Model button froze
 * without telling me why").
 *
 * ⚠ The unit tests next door cannot see this. `catalogSubscription.test.ts`
 * scripts its poll to park after the last scripted delta *specifically so the
 * loop cannot spin* — a reasonable thing to do to keep a test fast, and it
 * makes the runaway structurally unreachable there. The loop only exists once
 * the module is wired to a React effect, so this is where it has to be pinned.
 */
describe('ConfigContext catalogue subscription (#112)', () => {
  /** A faithful daemon: immediate when the caller is behind, parked when level. */
  function daemonAtRevision(revision: number) {
    return ({ query }: { query: { since: number } }) => {
      if (query.since >= revision) return new Promise(() => {});
      return Promise.resolve({
        data: {
          revision,
          changes: [
            {
              revision,
              reason: 'install' as const,
              extensions: [
                {
                  key: 'spokeagent',
                  name: 'SPOKEAgent',
                  change: 'added' as const,
                  enabled: true,
                  bundledSkillIds: [],
                },
              ],
              skills: [],
            },
          ],
          truncated: false,
        },
      });
    };
  }

  /**
   * ⚠ `mockResolvedValue` is the wrong tool for this file and it hid the bug.
   *
   * It hands every caller back the SAME object, so `setExtensionsList` receives
   * an array it is already holding, `Object.is` says equal, React bails out of
   * the update, and nothing downstream re-renders — which is precisely the
   * re-render the runaway is made of. A real response is parsed from a fresh
   * body on every call and is never reference-equal to the last one. Modelling
   * that is what makes this test able to fail.
   */
  function freshExtensionsResponse() {
    mocks.getExtensions.mockImplementation(() =>
      Promise.resolve({ data: { extensions: [], warnings: [] }, response: { status: 200 } })
    );
  }

  it('polls a moved catalogue a bounded number of times, not in a loop', async () => {
    freshExtensionsResponse();
    mocks.catalogChanges.mockImplementation(daemonAtRevision(4));

    renderProbe();

    // Let every microtask and timer settle. A correct subscription advances its
    // cursor to 4 and parks; a subscription that restarts re-asks from 0 and is
    // answered instantly, forever.
    for (let i = 0; i < 40; i += 1) {
      await act(async () => {
        await Promise.resolve();
      });
    }

    // Two is the honest ceiling for correct behaviour: the opening poll at
    // since=0, and the parked poll at since=4 that follows it. The assertion is
    // loose enough to survive an extra render, and orders of magnitude below
    // what the runaway produces.
    expect(mocks.catalogChanges.mock.calls.length).toBeLessThanOrEqual(4);
  });

  it('advances its cursor, so the poll after a change asks from the new revision', async () => {
    freshExtensionsResponse();
    mocks.catalogChanges.mockImplementation(daemonAtRevision(4));

    renderProbe();

    await waitFor(() => expect(mocks.catalogChanges.mock.calls.length).toBeGreaterThan(1));

    // The second poll must carry the revision the first one reported. Asking
    // from 0 again is the restart this whole describe exists to rule out.
    const second = mocks.catalogChanges.mock.calls[1][0] as { query: { since: number } };
    expect(second.query.since).toBe(4);
  });

  /**
   * The crash that told us where to look. Under a network failure the generated
   * client resolves with `{ error, response: undefined }` (see
   * `api/client/client.gen.ts` — it returns `response: undefined as any` from
   * the fetch catch), so reading `.status` off it throws a TypeError. Because
   * the subscription calls `refreshExtensions` with a bare `void`, that
   * TypeError became an unhandled rejection rather than a handled failure:
   *
   *   [UNHANDLED REJECTION] TypeError: Cannot read properties of undefined (reading 'status')
   *
   * A backend that cannot be reached must leave the cached list alone and say
   * so, never throw a type error out of a promise nobody is holding.
   */
  it('survives an extension refresh whose fetch never reached the daemon', async () => {
    mocks.getExtensions.mockResolvedValue({
      error: new TypeError('Failed to fetch'),
      response: undefined,
    });

    const unhandled: unknown[] = [];
    const onUnhandled = (event: PromiseRejectionEvent) => {
      unhandled.push(event.reason);
      event.preventDefault();
    };
    window.addEventListener('unhandledrejection', onUnhandled);

    try {
      renderProbe();
      for (let i = 0; i < 20; i += 1) {
        await act(async () => {
          await Promise.resolve();
        });
      }
    } finally {
      window.removeEventListener('unhandledrejection', onUnhandled);
    }

    expect(unhandled).toEqual([]);
  });
});

/**
 * F3 (provider QA, 2026-09-10). `BIOROUTER_PROVIDER` and `BIOROUTER_MODEL` are
 * what `/agent/start` binds a new chat to, so a write of either through this
 * context is announced to every window — each of which re-reads the pair. The
 * writers this catches are the ones that never pass through
 * `ModelAndProviderContext.changeModel`: onboarding's local and coding-agent
 * cards, Lead/Worker settings and Settings' reset, which until now left even
 * their own window's chip naming the previous model.
 */
describe('ConfigContext announces writes of the app-wide model selection (F3)', () => {
  function WriteProbe() {
    const { upsert, remove } = useConfig();
    const [result, setResult] = useState('idle');
    const run = (write: () => Promise<void>) => {
      setResult('pending');
      write().then(
        () => setResult('ok'),
        (error: unknown) => setResult(`failed: ${String(error)}`)
      );
    };
    return (
      <div>
        <output data-testid="write-result">{result}</output>
        <button
          type="button"
          onClick={() => run(() => upsert('BIOROUTER_PROVIDER', 'codex', false))}
        >
          Write provider
        </button>
        <button
          type="button"
          onClick={() => run(() => upsert('BIOROUTER_MODEL', 'gpt-6-astra', false))}
        >
          Write model
        </button>
        <button type="button" onClick={() => run(() => remove('BIOROUTER_MODEL', false))}>
          Remove model
        </button>
        <button type="button" onClick={() => run(() => upsert('BIOROUTER_MODE', 'auto', false))}>
          Write mode
        </button>
      </div>
    );
  }

  let nudges = 0;
  let unsubscribe: () => void = () => {};

  beforeEach(async () => {
    nudges = 0;
    const { subscribeAppModelSelectionChanges } = await import('../utils/sessionBindingSync');
    unsubscribe = subscribeAppModelSelectionChanges(() => {
      nudges += 1;
    });
    mocks.removeConfig.mockResolvedValue({ data: {} });
  });

  afterEach(() => unsubscribe());

  const renderWriteProbe = () =>
    render(
      <ConfigProvider>
        <WriteProbe />
      </ConfigProvider>
    );

  it.each([['Write provider'], ['Write model'], ['Remove model']])(
    '%s announces, once the write has resolved',
    async (button) => {
      renderWriteProbe();
      fireEvent.click(screen.getByRole('button', { name: button }));
      await waitFor(() => expect(screen.getByTestId('write-result')).toHaveTextContent('ok'));
      expect(nudges).toBe(1);
    }
  );

  it('says nothing about a key a new chat does not bind', async () => {
    renderWriteProbe();
    fireEvent.click(screen.getByRole('button', { name: 'Write mode' }));
    await waitFor(() => expect(screen.getByTestId('write-result')).toHaveTextContent('ok'));
    expect(nudges).toBe(0);
  });

  /** A refused write moved nothing, so there is nothing to re-read. */
  it('says nothing when the write was refused', async () => {
    mocks.upsertConfig.mockRejectedValue(new Error('409 Conflict'));
    renderWriteProbe();
    fireEvent.click(screen.getByRole('button', { name: 'Write provider' }));
    await waitFor(() =>
      expect(screen.getByTestId('write-result')).toHaveTextContent('failed: Error: 409 Conflict')
    );
    expect(nudges).toBe(0);
  });
});
