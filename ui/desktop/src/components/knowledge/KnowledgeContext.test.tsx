import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { KnowledgeProvider, SELECTION_NOT_SAVED_TITLE, useKnowledge } from './KnowledgeContext';
import { useKnowledgeBases } from './hooks/useKnowledgeBases';
import { reachGatedGetActive, SESSION_OUT_OF_REACH, USER_ACTION_KEY } from '../../test/reachGate';

/** A promise the test resolves by hand, so "after the response settled" is a fact, not a race. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** Resolve/reject a pending sync and let React apply every queued state update. */
async function settle(action: () => void) {
  await act(async () => {
    action();
    await Promise.resolve();
    await Promise.resolve();
  });
}

const mocks = vi.hoisted(() => ({
  listBases: vi.fn(),
  getActive: vi.fn(),
  setActive: vi.fn(),
  createBase: vi.fn(),
  deleteBase: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('../../api', () => ({
  listBases: mocks.listBases,
  getActive: mocks.getActive,
  setActive: mocks.setActive,
  createBase: mocks.createBase,
  deleteBase: mocks.deleteBase,
}));

vi.mock('../../toasts', () => ({ toastError: mocks.toastError }));

function base(id: string) {
  return { id, name: id, color: '#cf6d47', created_at: '', schema_version: 1 };
}

type Selection = {
  kb_ids: string[];
  primary_kb: string | null;
  active_kb: string | null;
  hidden_kbs: string[];
};

/**
 * What the daemon answers at each scope. `GET /active` with a `session_id` is
 * the chat's resolved selection; without one it is the machine-wide default —
 * the thing a chat inherits when it holds no pointer of its own. Tests move
 * these two apart to describe a chat that has overridden the default.
 */
const daemon: { session: Selection; machine: Selection } = {
  session: { kb_ids: [], primary_kb: null, active_kb: null, hidden_kbs: [] },
  machine: { kb_ids: [], primary_kb: null, active_kb: null, hidden_kbs: [] },
};

function Probe() {
  const {
    primaryKbId,
    hiddenKbIds,
    visibleBases,
    loading,
    basesError,
    defaultPrimaryKb,
    canFollowDefaultPrimary,
    followDefaultPrimary,
    refreshDefaultPrimary,
    setPrimaryKbId,
    toggleKbHidden,
    refresh,
  } = useKnowledge();
  const { remove } = useKnowledgeBases();
  return (
    <div>
      <span data-testid="primary">{primaryKbId ?? 'none'}</span>
      <span data-testid="hidden">{hiddenKbIds.join(',') || 'none'}</span>
      <span data-testid="visible">{visibleBases.map((b) => b.id).join(',') || 'none'}</span>
      <span data-testid="loading">{loading ? 'loading' : 'idle'}</span>
      <span data-testid="bases-error">{basesError ?? 'none'}</span>
      <span data-testid="default-primary">{defaultPrimaryKb?.id ?? 'none'}</span>
      <span data-testid="can-follow-default">{canFollowDefaultPrimary ? 'yes' : 'no'}</span>
      <button type="button" onClick={() => void refresh()}>
        refresh
      </button>
      <button type="button" onClick={() => void refreshDefaultPrimary()}>
        read the default
      </button>
      <button type="button" onClick={() => followDefaultPrimary()}>
        follow the default
      </button>
      <button type="button" onClick={() => setPrimaryKbId('beta')}>
        make beta primary
      </button>
      <button type="button" onClick={() => setPrimaryKbId('alpha')}>
        make alpha primary
      </button>
      <button type="button" onClick={() => toggleKbHidden('alpha')}>
        toggle alpha
      </button>
      <button type="button" onClick={() => void remove('alpha')}>
        delete alpha
      </button>
    </div>
  );
}

function renderProvider(sessionId: string | null = 'chat-1') {
  return render(
    <KnowledgeProvider sessionId={sessionId}>
      <Probe />
    </KnowledgeProvider>
  );
}

/** Render, wait for the hydrate, then read the machine-wide default. */
async function renderWithDefault(sessionId: string | null = 'chat-1') {
  const result = renderProvider(sessionId);
  await waitFor(() => expect(mocks.getActive).toHaveBeenCalled());
  await userEvent.click(screen.getByRole('button', { name: 'read the default' }));
  await settle(() => {});
  return result;
}

beforeEach(() => {
  localStorage.clear();
  vi.clearAllMocks();
  mocks.listBases.mockResolvedValue({ data: [base('alpha'), base('beta')] });
  daemon.session = {
    kb_ids: ['alpha'],
    primary_kb: 'alpha',
    active_kb: 'alpha',
    hidden_kbs: ['beta'],
  };
  daemon.machine = {
    kb_ids: ['alpha', 'beta'],
    primary_kb: 'alpha',
    active_kb: 'alpha',
    hidden_kbs: [],
  };
  mocks.getActive.mockImplementation((options?: { query?: { session_id?: string } }) =>
    Promise.resolve({ data: options?.query?.session_id ? daemon.session : daemon.machine })
  );
  mocks.setActive.mockResolvedValue({
    data: { kb_ids: ['alpha', 'beta'], primary_kb: 'beta', active_kb: 'beta', hidden_kbs: [] },
  });
  mocks.deleteBase.mockResolvedValue({});
});

describe('KnowledgeContext', () => {
  it('renders Soul as the primary in a fresh default selection', async () => {
    mocks.listBases.mockResolvedValue({ data: [base('soul')] });
    daemon.session = {
      kb_ids: ['soul'],
      primary_kb: 'soul',
      active_kb: 'soul',
      hidden_kbs: [],
    };

    renderProvider();

    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('soul'));
    expect(screen.getByTestId('visible')).toHaveTextContent('soul');
  });

  it('hydrates the primary and the set from the daemon', async () => {
    renderProvider();
    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));
    expect(screen.getByTestId('hidden')).toHaveTextContent('beta');
  });

  // The invariant, at the UI edge: the primary must be a member of the set, so
  // "make primary" on a base that is toggled off is ONE request that does both
  // and is validated by the daemon against the state it produces.
  it('makes a base primary and adds it to the chat in the same request', async () => {
    renderProvider();
    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));

    await userEvent.click(screen.getByRole('button', { name: 'make beta primary' }));

    await waitFor(() => expect(mocks.setActive).toHaveBeenCalled());
    const calls = mocks.setActive.mock.calls;
    const body = calls[calls.length - 1]?.[0]?.body;
    expect(body.primary_kb).toBe('beta');
    expect(body.hidden_kbs).toEqual([]);
    expect(body.session_id).toBe('chat-1');
  });

  // The promote/clear rule lives in the daemon. If the UI re-derived it, the
  // two would drift and the chat chip would disagree with the model.
  it('adopts the primary the daemon reports back', async () => {
    mocks.setActive.mockResolvedValue({
      data: { kb_ids: ['alpha'], primary_kb: 'alpha', active_kb: 'alpha', hidden_kbs: ['beta'] },
    });
    renderProvider();
    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));

    await userEvent.click(screen.getByRole('button', { name: 'make beta primary' }));

    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));
  });

  // Toggling the primary's own membership off is the case the repair exists
  // for. A set-only edit must therefore state NO primary: echoing the current
  // one back would be a primary outside the resulting set, which the daemon
  // rejects with a 400 — the badge would vanish instead of moving.
  it('states no primary on a set-only edit, so the daemon can promote', async () => {
    mocks.setActive.mockResolvedValue({
      data: { kb_ids: ['beta'], primary_kb: 'beta', active_kb: 'beta', hidden_kbs: ['alpha'] },
    });
    renderProvider();
    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));

    await userEvent.click(screen.getByRole('button', { name: 'toggle alpha' }));

    await waitFor(() => expect(mocks.setActive).toHaveBeenCalled());
    const calls = mocks.setActive.mock.calls;
    const body = calls[calls.length - 1]?.[0]?.body;
    expect(body.primary_kb).toBeUndefined();
    expect(body.clear_primary).toBe(false);
    expect(body.hidden_kbs).toEqual(['alpha', 'beta']);
    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('beta'));
  });

  // The primary is the KB-less write target. Between the click that removes it
  // from the chat and the daemon's repair there must be no window in which the
  // renderer still names it — IngestPanel passes `primaryKbId` explicitly, so a
  // stale one aims a digest at a base this session no longer includes.
  it('never keeps a primary the user just removed from the set', async () => {
    const pending = deferred<unknown>();
    renderProvider();
    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));

    mocks.setActive.mockReturnValue(pending.promise);
    await userEvent.click(screen.getByRole('button', { name: 'toggle alpha' }));
    await waitFor(() => expect(mocks.setActive).toHaveBeenCalled());

    expect(screen.getByTestId('hidden').textContent).toBe('alpha,beta');
    expect(screen.getByTestId('primary').textContent).toBe('none');

    await settle(() =>
      pending.resolve({
        data: { kb_ids: ['beta'], primary_kb: 'beta', active_kb: 'beta', hidden_kbs: ['alpha'] },
      })
    );

    // …and then the whole repair is adopted, set included, so the primary is a
    // member of the visible set again rather than of a set only the daemon has.
    expect(screen.getByTestId('primary').textContent).toBe('beta');
    expect(screen.getByTestId('hidden').textContent).toBe('alpha');
    expect(screen.getByTestId('visible').textContent).toBe('beta');
  });

  // Mixed versions: a new renderer against a daemon that predates `primary_kb`.
  // Its POST answer carries only the deprecated `active_kb` mirror, and reading
  // just `primary_kb` turns a *successful* write into "there is no primary" —
  // which the design forbids inventing back, so the pointer would stay lost.
  // Every other reader already falls back to the mirror; this one must too.
  it('reads the deprecated active_kb mirror out of a successful POST', async () => {
    const pending = deferred<unknown>();
    renderProvider();
    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));

    mocks.setActive.mockReturnValue(pending.promise);
    await userEvent.click(screen.getByRole('button', { name: 'make beta primary' }));
    await waitFor(() => expect(mocks.setActive).toHaveBeenCalled());

    await settle(() =>
      pending.resolve({
        data: { kb_ids: ['alpha', 'beta'], active_kb: 'beta', hidden_kbs: [] },
      })
    );

    expect(screen.getByTestId('primary')).toHaveTextContent('beta');
  });

  // The optimistic value is a guess about what the daemon will do. When the
  // write does not land, keeping the guess leaves the chip, the Knowledge view
  // and the ingest target describing a selection the daemon never applied — and
  // nothing later corrects it. Re-read the truth instead.
  it('re-reads the daemon selection when the write is rejected', async () => {
    const pending = deferred<unknown>();
    renderProvider();
    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));
    expect(mocks.getActive).toHaveBeenCalledTimes(1);

    mocks.setActive.mockReturnValue(pending.promise);
    await userEvent.click(screen.getByRole('button', { name: 'make beta primary' }));
    expect(screen.getByTestId('primary').textContent).toBe('beta');

    await settle(() => pending.reject(new Error('network down')));

    expect(mocks.getActive).toHaveBeenCalledTimes(2);
    expect(screen.getByTestId('primary').textContent).toBe('alpha');
    expect(screen.getByTestId('hidden').textContent).toBe('beta');
    // …and the person who clicked is told the click did not land.
    expect(mocks.toastError).toHaveBeenCalledWith({
      title: SELECTION_NOT_SAVED_TITLE,
      msg: 'network down',
    });
  });

  // Same divergence by the other door: the client resolves, but with an error
  // envelope instead of a selection. That is a write that did not land.
  it('re-reads the daemon selection when the write returns no selection', async () => {
    const pending = deferred<unknown>();
    renderProvider();
    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));

    mocks.setActive.mockReturnValue(pending.promise);
    await userEvent.click(screen.getByRole('button', { name: 'make beta primary' }));

    await settle(() => pending.resolve({ error: { message: 'primary_kb is not a member' } }));

    expect(mocks.getActive).toHaveBeenCalledTimes(2);
    expect(screen.getByTestId('primary').textContent).toBe('alpha');
    expect(screen.getByTestId('hidden').textContent).toBe('beta');
    expect(mocks.toastError).toHaveBeenCalledWith({
      title: SELECTION_NOT_SAVED_TITLE,
      msg: 'primary_kb is not a member',
    });
  });

  // Two clicks, two writes, answers out of order. The older answer describes a
  // selection the user has already moved on from; adopting it silently undoes
  // the newer click.
  it('ignores a superseded response that lands after a newer one', async () => {
    const first = deferred<unknown>();
    const second = deferred<unknown>();
    renderProvider();
    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));

    mocks.setActive.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    await userEvent.click(screen.getByRole('button', { name: 'make beta primary' }));
    await userEvent.click(screen.getByRole('button', { name: 'make alpha primary' }));
    expect(mocks.setActive).toHaveBeenCalledTimes(2);

    await settle(() =>
      second.resolve({
        data: {
          kb_ids: ['alpha', 'beta'],
          primary_kb: 'alpha',
          active_kb: 'alpha',
          hidden_kbs: [],
        },
      })
    );
    expect(screen.getByTestId('primary').textContent).toBe('alpha');

    await settle(() =>
      first.resolve({
        data: { kb_ids: ['beta'], primary_kb: 'beta', active_kb: 'beta', hidden_kbs: ['alpha'] },
      })
    );
    expect(screen.getByTestId('primary').textContent).toBe('alpha');
    expect(screen.getByTestId('hidden').textContent).toBe('none');
  });

  // The prune drops ids naming bases that no longer exist. An empty base list is
  // not evidence that nothing exists — on mount it only means the list has not
  // arrived — and pruning against it writes the session's whole set away.
  it('does not prune the set before the base list has arrived', async () => {
    mocks.listBases.mockReturnValue(new Promise(() => {}));
    renderProvider();
    await waitFor(() => expect(mocks.getActive).toHaveBeenCalled());
    await settle(() => {});

    expect(mocks.setActive).not.toHaveBeenCalled();
    expect(screen.getByTestId('hidden').textContent).toBe('beta');
  });

  // A pointer at a base the list no longer holds must not be SHOWN — the view,
  // the ingest target and the graph would all aim at a base that is gone — and
  // must not be WRITTEN back as "no primary" either. That write used to happen
  // here: a durable, session-scoped override derived from whatever list this
  // renderer held, installed in a chat that may only have inherited the pointer
  // and that the daemon had deliberately left alone (D2; QA 2026-09-10 F14).
  it('hides a primary whose base is gone without writing a durable clear', async () => {
    mocks.listBases.mockResolvedValue({ data: [] });
    daemon.session.hidden_kbs = [];

    renderProvider();

    await waitFor(() => expect(mocks.listBases).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('none'));
    await settle(() => {});
    expect(mocks.setActive).not.toHaveBeenCalled();
    // …and the daemon's answer is what `localStorage` keeps: the renderer did
    // not invent a different one to persist.
    expect(localStorage.getItem('knowledge_active_kb:chat-1')).toBe('alpha');
  });

  // Same, by the other door: a list request that fails is not a list of zero
  // bases. Emptying the list on failure hands the prune the same false evidence.
  it('keeps the base list when a refresh fails', async () => {
    renderProvider();
    await waitFor(() => expect(screen.getByTestId('visible').textContent).toBe('alpha'));

    mocks.listBases.mockRejectedValue(new Error('daemon down'));
    await userEvent.click(screen.getByRole('button', { name: 'refresh' }));
    await settle(() => {});

    expect(screen.getByTestId('visible').textContent).toBe('alpha');
    expect(screen.getByTestId('hidden').textContent).toBe('beta');
    expect(mocks.setActive).not.toHaveBeenCalled();
  });

  // …and having kept it, the provider must say that it is stale. `loading` goes
  // false either way, so without this a consumer reads "the list is settled and
  // this base is not in it" out of a read that never happened — which is how
  // IngestPanel came to resolve a model for a base it had never seen.
  it('reports that the base list could not be read, and stops once one lands', async () => {
    mocks.listBases.mockRejectedValue(new Error('daemon down'));
    renderProvider();

    await waitFor(() => expect(screen.getByTestId('loading').textContent).toBe('idle'));
    expect(screen.getByTestId('bases-error').textContent).toBe('daemon down');

    mocks.listBases.mockResolvedValue({ data: [base('alpha'), base('beta')] });
    await userEvent.click(screen.getByRole('button', { name: 'refresh' }));
    await settle(() => {});

    expect(screen.getByTestId('bases-error').textContent).toBe('none');
    expect(screen.getByTestId('visible').textContent).toBe('alpha');
  });

  // A rejection carrying no message is still a failed read. Reporting it as an
  // empty string makes every `if (basesError)` consumer read "all fine".
  it('reports a failure that arrived without a message', async () => {
    mocks.listBases.mockRejectedValue(new Error(''));
    renderProvider();

    await waitFor(() => expect(screen.getByTestId('loading').textContent).toBe('idle'));
    expect(screen.getByTestId('bases-error').textContent).not.toBe('none');
    expect(screen.getByTestId('bases-error').textContent).not.toBe('');
  });

  // QA 2026-09-10 F14. Around the refused reads (see 'a private chat' below), the
  // renderer made selection writes nobody asked for, which is how a click could
  // look saved and not be. It now writes only what a person clicked, persists
  // only what the daemon confirmed, and says so when a write does not land.
  describe('writes only what the daemon confirmed', () => {
    // The renderer believed the write succeeded while the daemon had refused
    // it: `localStorage` took the guess before the POST went out, and nothing on
    // screen ever said the click had not landed.
    it('says a refused write did not land, and keeps localStorage on what the daemon confirmed', async () => {
      const write = deferred<unknown>();
      renderProvider();
      await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));
      expect(localStorage.getItem('knowledge_active_kb:chat-1')).toBe('alpha');

      mocks.setActive.mockReturnValue(write.promise);
      // …and the recovery read cannot reach the daemon either.
      mocks.getActive.mockRejectedValue(new Error('Failed to fetch'));
      await userEvent.click(screen.getByRole('button', { name: 'make beta primary' }));

      // Optimistic on screen, never in storage.
      expect(screen.getByTestId('primary').textContent).toBe('beta');
      expect(localStorage.getItem('knowledge_active_kb:chat-1')).toBe('alpha');

      const listsBefore = mocks.listBases.mock.calls.length;
      await settle(() => write.resolve({ error: SESSION_OUT_OF_REACH }));

      await waitFor(() => expect(mocks.toastError).toHaveBeenCalledTimes(1));
      expect(mocks.toastError).toHaveBeenCalledWith({
        title: SELECTION_NOT_SAVED_TITLE,
        msg: 'That chat is private, or there is no chat with that id.',
      });
      // The list is re-read as well: a choice is most often refused because
      // the base it named went away somewhere else.
      expect(mocks.listBases.mock.calls.length).toBeGreaterThan(listsBefore);
      await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));
      expect(screen.getByTestId('hidden').textContent).toBe('beta');
      expect(localStorage.getItem('knowledge_active_kb:chat-1')).toBe('alpha');
      expect(localStorage.getItem('knowledge_hidden_kbs:chat-1')).toBe('["beta"]');
    });

    // A base the chat already uses needs no set edit. Echoing the resolved
    // hidden list back installed a session-scoped override on a chat that was
    // inheriting the machine-wide one.
    it('makes a base already in the chat primary without re-sending the set', async () => {
      daemon.session = {
        kb_ids: ['alpha', 'beta'],
        primary_kb: 'alpha',
        active_kb: 'alpha',
        hidden_kbs: [],
      };
      renderProvider();
      await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));

      await userEvent.click(screen.getByRole('button', { name: 'make beta primary' }));

      await waitFor(() => expect(mocks.setActive).toHaveBeenCalled());
      const body = mocks.setActive.mock.calls[0]?.[0]?.body;
      expect(body.primary_kb).toBe('beta');
      expect(body.hidden_kbs).toBeUndefined();
    });

    // The delete IS the repair (D2): the daemon clears every pointer that named
    // the base. The renderer used to write `clear_primary` on top of it — a
    // durable "no primary" in a chat that may only have inherited the pointer.
    it("follows the daemon's repair after a delete instead of writing a clear", async () => {
      renderProvider();
      await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));
      // What the daemon holds once `alpha` is gone: it cleared this chat's pin.
      mocks.listBases.mockResolvedValue({ data: [base('beta')] });
      daemon.session = { kb_ids: [], primary_kb: null, active_kb: null, hidden_kbs: ['beta'] };
      const readsBefore = mocks.getActive.mock.calls.length;

      await userEvent.click(screen.getByRole('button', { name: 'delete alpha' }));

      await waitFor(() =>
        expect(mocks.deleteBase).toHaveBeenCalledWith(
          expect.objectContaining({ path: { id: 'alpha' } })
        )
      );
      await waitFor(() => expect(mocks.getActive.mock.calls.length).toBeGreaterThan(readsBefore));
      await waitFor(() => expect(localStorage.getItem('knowledge_active_kb:chat-1')).toBeNull());
      expect(screen.getByTestId('primary').textContent).toBe('none');
      expect(mocks.setActive).not.toHaveBeenCalled();
    });

    // `refresh` now re-reads the selection, so it must never land on top of a
    // click: a read answered before the write commits describes the selection
    // the user just moved away from.
    it('never lets a background refresh overwrite a write that is still out', async () => {
      renderProvider();
      await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));

      // A refresh whose selection read is answered late, with the old selection…
      const staleRead = deferred<unknown>();
      mocks.getActive.mockReturnValueOnce(staleRead.promise);
      await userEvent.click(screen.getByRole('button', { name: 'refresh' }));

      // …then a click.
      const write = deferred<unknown>();
      mocks.setActive.mockReturnValue(write.promise);
      await userEvent.click(screen.getByRole('button', { name: 'make beta primary' }));

      // A refresh while the write is out does not even ask.
      const readsDuringWrite = mocks.getActive.mock.calls.length;
      await userEvent.click(screen.getByRole('button', { name: 'refresh' }));
      await settle(() => {});
      expect(mocks.getActive.mock.calls.length).toBe(readsDuringWrite);

      await settle(() => staleRead.resolve({ data: daemon.session }));
      expect(screen.getByTestId('primary').textContent).toBe('beta');

      await settle(() =>
        write.resolve({
          data: {
            kb_ids: ['alpha', 'beta'],
            primary_kb: 'beta',
            active_kb: 'beta',
            hidden_kbs: [],
          },
        })
      );
      expect(screen.getByTestId('primary').textContent).toBe('beta');
      expect(localStorage.getItem('knowledge_active_kb:chat-1')).toBe('beta');
    });
  });

  // The fourth intent. `clear` writes a *durable* "this chat has no primary",
  // and deleting the base a chat had pinned installs exactly that — so without
  // a way to drop the chat's own pointer, such a chat could never follow the
  // machine-wide default again from the GUI.
  describe('following the default again', () => {
    it('reads the machine-wide default from the unscoped selection', async () => {
      daemon.machine.primary_kb = 'beta';
      daemon.machine.active_kb = 'beta';
      await renderWithDefault();

      expect(screen.getByTestId('default-primary').textContent).toBe('beta');
      const scopes = mocks.getActive.mock.calls.map(
        (call: unknown[]) =>
          (call[0] as { query?: { session_id?: string } } | undefined)?.query?.session_id ?? null
      );
      expect(scopes).toContain('chat-1');
      expect(scopes).toContain(null);
    });

    // A chat that is already showing the default has nothing to inherit, so
    // offering it the gesture is noise.
    it('offers nothing to a chat that is already on the default', async () => {
      await renderWithDefault();

      expect(screen.getByTestId('primary').textContent).toBe('alpha');
      expect(screen.getByTestId('can-follow-default').textContent).toBe('no');
    });

    it('offers the default to a chat that pinned something else', async () => {
      daemon.session = {
        kb_ids: ['alpha', 'beta'],
        primary_kb: 'beta',
        active_kb: 'beta',
        hidden_kbs: [],
      };
      await renderWithDefault();

      expect(screen.getByTestId('can-follow-default').textContent).toBe('yes');
      expect(screen.getByTestId('default-primary').textContent).toBe('alpha');
    });

    // The state a delete leaves behind: an explicit "no primary here" that
    // survives every other gesture. This is the one the affordance exists for.
    it('offers the default to a chat left with no primary at all', async () => {
      daemon.session = {
        kb_ids: ['alpha', 'beta'],
        primary_kb: null,
        active_kb: null,
        hidden_kbs: [],
      };
      await renderWithDefault();

      expect(screen.getByTestId('primary').textContent).toBe('none');
      expect(screen.getByTestId('can-follow-default').textContent).toBe('yes');
    });

    // The primary must be a member of the set, so inheriting a default this
    // chat has left out resolves to *no* primary — the offer would do nothing
    // the user can see. The membership switch is the gesture for that, and once
    // it is on the offer appears.
    it('does not offer a default this chat has left out of its set', async () => {
      daemon.session = {
        kb_ids: ['alpha'],
        primary_kb: 'alpha',
        active_kb: 'alpha',
        hidden_kbs: ['beta'],
      };
      daemon.machine.primary_kb = 'beta';
      daemon.machine.active_kb = 'beta';
      await renderWithDefault();

      expect(screen.getByTestId('default-primary').textContent).toBe('beta');
      expect(screen.getByTestId('can-follow-default').textContent).toBe('no');
    });

    // The three primary gestures are mutually exclusive on the wire: two of
    // them in one body is a 400 naming both fields, not a precedence rule.
    // The set travels separately and is left alone — a gesture that means
    // "stop overriding here" must not install a different override on the way.
    it('sends inherit_primary alone', async () => {
      daemon.session = {
        kb_ids: ['alpha', 'beta'],
        primary_kb: 'beta',
        active_kb: 'beta',
        hidden_kbs: [],
      };
      await renderWithDefault();

      await userEvent.click(screen.getByRole('button', { name: 'follow the default' }));

      await waitFor(() => expect(mocks.setActive).toHaveBeenCalled());
      const calls = mocks.setActive.mock.calls;
      const body = calls[calls.length - 1]?.[0]?.body;
      expect(body.inherit_primary).toBe(true);
      expect(body.primary_kb).toBeUndefined();
      expect(body.clear_primary).toBe(false);
      expect(body.hidden_kbs).toBeUndefined();
      expect(body.session_id).toBe('chat-1');
    });

    // Which base "the default" resolves to is the daemon's to decide — it
    // re-reads the machine pointer and filters it through this chat's set. The
    // renderer adopts that answer rather than assuming its own cached default.
    it('adopts whatever primary the daemon reports back', async () => {
      daemon.session = {
        kb_ids: ['alpha', 'beta'],
        primary_kb: 'beta',
        active_kb: 'beta',
        hidden_kbs: [],
      };
      mocks.setActive.mockResolvedValue({
        data: {
          kb_ids: ['alpha', 'beta'],
          primary_kb: 'alpha',
          active_kb: 'alpha',
          hidden_kbs: [],
        },
      });
      await renderWithDefault();

      await userEvent.click(screen.getByRole('button', { name: 'follow the default' }));

      await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));
      expect(screen.getByTestId('can-follow-default').textContent).toBe('no');
    });

    // A write that never landed must not leave the chat looking like it
    // inherited: the pointer is never guessed forward, and the failure is
    // recovered by re-reading the truth, which still offers the way back.
    it('does not claim the chat inherited when the write fails', async () => {
      daemon.session = {
        kb_ids: ['alpha', 'beta'],
        primary_kb: 'beta',
        active_kb: 'beta',
        hidden_kbs: [],
      };
      const pending = deferred<unknown>();
      await renderWithDefault();
      const readsBefore = mocks.getActive.mock.calls.length;

      mocks.setActive.mockReturnValue(pending.promise);
      await userEvent.click(screen.getByRole('button', { name: 'follow the default' }));
      expect(screen.getByTestId('primary').textContent).toBe('beta');

      await settle(() => pending.reject(new Error('network down')));

      expect(mocks.getActive.mock.calls.length).toBe(readsBefore + 1);
      expect(screen.getByTestId('primary').textContent).toBe('beta');
      expect(screen.getByTestId('can-follow-default').textContent).toBe('yes');
    });

    // This is intentionally a chat-only affordance. Machine scope can restore
    // Soul through settings/CLI/API, but there is no chat override to drop.
    it('does not offer the chat-only follow control outside a chat', async () => {
      daemon.machine.primary_kb = 'beta';
      daemon.machine.active_kb = 'beta';
      await renderWithDefault(null);

      expect(screen.getByTestId('can-follow-default').textContent).toBe('no');
      await userEvent.click(screen.getByRole('button', { name: 'follow the default' }));
      await settle(() => {});
      expect(mocks.setActive).not.toHaveBeenCalled();
    });
  });

  // Issue #56 Task 58. `GET /knowledge/active` naming a PRIVATE chat is on the
  // reach gate's list, and the desktop app gets through it the way `setActive`
  // already does: with the user's proof. The reads carried none, so the daemon
  // refused every private chat's selection — whatever model was bound — and the
  // chip fell back to whatever this renderer had cached, which the daemon may
  // have moved past (the agent's `kb_set_active`, the CLI, another window).
  describe('a private chat', () => {
    let savedElectron: unknown;

    beforeEach(() => {
      savedElectron = (window as { electron?: unknown }).electron;
      Object.assign(window, {
        electron: { getUserActionKey: vi.fn(async () => USER_ACTION_KEY) },
      });
      mocks.getActive.mockImplementation(
        reachGatedGetActive(['chat-1'], (sessionId) =>
          sessionId ? daemon.session : daemon.machine
        )
      );
    });

    afterEach(() => {
      Object.assign(window, { electron: savedElectron });
    });

    it('hydrates its selection from the daemon, not from what this renderer cached', async () => {
      localStorage.setItem('knowledge_active_kb:chat-1', 'beta');
      localStorage.setItem('knowledge_hidden_kbs:chat-1', '[]');

      renderProvider();

      await waitFor(() => expect(screen.getByTestId('primary').textContent).toBe('alpha'));
      expect(screen.getByTestId('hidden').textContent).toBe('beta');
      expect(mocks.getActive).toHaveBeenCalledWith(
        expect.objectContaining({
          query: { session_id: 'chat-1' },
          headers: { 'X-User-Action': USER_ACTION_KEY },
        })
      );
    });

    // Seeded to match the daemon, so the first read cannot be what puts the
    // selection back: only the recovery read can.
    it('re-reads its selection with the proof when a write does not land', async () => {
      localStorage.setItem('knowledge_active_kb:chat-1', 'alpha');
      localStorage.setItem('knowledge_hidden_kbs:chat-1', '["beta"]');
      const pending = deferred<unknown>();
      renderProvider();
      await waitFor(() => expect(mocks.getActive).toHaveBeenCalledTimes(1));
      await settle(() => {});

      mocks.setActive.mockReturnValue(pending.promise);
      await userEvent.click(screen.getByRole('button', { name: 'make beta primary' }));
      expect(screen.getByTestId('primary').textContent).toBe('beta');

      await settle(() => pending.reject(new Error('network down')));

      await waitFor(() => expect(mocks.getActive).toHaveBeenCalledTimes(2));
      expect(mocks.getActive.mock.calls[1]?.[0]).toMatchObject({
        query: { session_id: 'chat-1' },
        headers: { 'X-User-Action': USER_ACTION_KEY },
      });
      await waitFor(() => expect(screen.getByTestId('primary').textContent).toBe('alpha'));
      expect(screen.getByTestId('hidden').textContent).toBe('beta');
    });

    // What the refused read cost, measured in the running app on 2026-09-11:
    // the chip showed a hidden base as switched on, and one click on another
    // base wrote that stale set back — the hidden base was in the chat again.
    // A set-only edit is only as right as the set it starts from.
    it('toggles from the set the daemon holds, so a toggle cannot re-expose a hidden base', async () => {
      localStorage.setItem('knowledge_hidden_kbs:chat-1', '[]');
      renderProvider();
      await waitFor(() => expect(mocks.getActive).toHaveBeenCalled());
      // Let the hydrate's answer land, whichever way the daemon answered it.
      await act(async () => {
        await Promise.allSettled(mocks.getActive.mock.results.map((result) => result.value));
      });

      await userEvent.click(screen.getByRole('button', { name: 'toggle alpha' }));

      await waitFor(() => expect(mocks.setActive).toHaveBeenCalled());
      const calls = mocks.setActive.mock.calls;
      expect(calls[calls.length - 1]?.[0]?.body.hidden_kbs).toEqual(['alpha', 'beta']);
    });
  });
});
