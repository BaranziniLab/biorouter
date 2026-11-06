import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CREW_READ_CONSENT_EVENT } from '../../toolCallCopy';
import { readsCopy } from '../account/copy';
import { CrewHttpError } from '../crewApi';
import { useChatCrewAccess } from './chatCrewAccess';
import { ChatCrewAccessBar } from './ChatCrewAccessBar';
import { connection, grantRow } from './testing';

/**
 * A chat with no Crew grant that may read a workspace as the person (account ownership AO-12's
 * read consent) says so above its composer, with Revoke: the consent lasts until it is taken back,
 * and before this only the `/crew` pane listed it (live QA).
 */

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => vi.fn() };
});

interface Daemon {
  grants?: () => unknown[];
  reads: () => unknown[];
  revoke?: () => unknown;
}

function installDaemon(daemon: Daemon) {
  mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
    if (path === '/connections') return { connections: [connection] };
    if (path === '/connections/conn-1/grants' && method === 'GET')
      return { grants: daemon.grants?.() ?? [], person_reads: daemon.reads() };
    if (path === '/sessions/chat-1/reads/conn-1' && method === 'DELETE')
      return daemon.revoke ? daemon.revoke() : {};
    return {};
  });
}

const consent = { session_id: 'chat-1', connection_id: 'conn-1', granted_at: 1_700_000_000 };

const grantReads = () =>
  mocks.crewHttp.mock.calls.filter(
    ([path, method = 'GET']) => path === '/connections/conn-1/grants' && method === 'GET'
  ).length;

function Chat() {
  const access = useChatCrewAccess('chat-1');
  return (
    <div>
      <ChatCrewAccessBar access={access} chatTitle="Plot review" />
      <textarea data-testid="chat-input" aria-label="Message" />
    </div>
  );
}

function renderChat() {
  return render(
    <MemoryRouter>
      <Chat />
    </MemoryRouter>
  );
}

beforeEach(() => {
  mocks.crewHttp.mockReset();
});
afterEach(() => {
  vi.useRealTimers();
});

describe('a chat that reads a workspace as you, with no grant (AO-12)', () => {
  it('says so in the chat, and Revoke takes it back with focus kept', async () => {
    let reads = [consent];
    installDaemon({ reads: () => reads });
    renderChat();
    expect(await screen.findByText(readsCopy.reads('Fixture'))).toBeInTheDocument();
    const revoke = screen.getByRole('button', { name: readsCopy.revokeLabel('Fixture') });
    revoke.focus();
    reads = [];
    fireEvent.click(revoke);
    expect(await screen.findByText(readsCopy.revoked('Fixture'))).toBeInTheDocument();
    expect(
      mocks.crewHttp.mock.calls.some(
        ([path, method]) => path === '/sessions/chat-1/reads/conn-1' && method === 'DELETE'
      )
    ).toBe(true);
    expect(screen.queryByText(readsCopy.reads('Fixture'))).toBeNull();
    // Its button went with it: focus is on the note's Dismiss, never on the page.
    const dismiss = screen.getByRole('button', { name: readsCopy.dismiss });
    await waitFor(() => expect(document.activeElement).toBe(dismiss));
    fireEvent.click(dismiss);
    await waitFor(() => expect(document.activeElement).toBe(screen.getByTestId('chat-input')));
    expect(screen.queryByTestId('crew-chat-reads-bar')).toBeNull();
  });

  it('keeps the row and says so when the revoke is refused', async () => {
    installDaemon({
      reads: () => [consent],
      revoke: () => {
        throw new CrewHttpError('No.', 503, 'crew_not_sent');
      },
    });
    renderChat();
    fireEvent.click(await screen.findByRole('button', { name: readsCopy.revokeLabel('Fixture') }));
    expect(await screen.findByRole('alert')).toHaveTextContent(readsCopy.failed);
    const revoke = screen.getByRole('button', { name: readsCopy.revokeLabel('Fixture') });
    await waitFor(() => expect(document.activeElement).toBe(revoke));
  });

  it('shows nothing for a chat with no consent and no grant', async () => {
    installDaemon({ reads: () => [{ ...consent, session_id: 'another-chat' }] });
    renderChat();
    await waitFor(() => expect(grantReads()).toBe(1));
    expect(screen.queryByTestId('crew-chat-reads-bar')).toBeNull();
  });

  it('leaves a chat with a grant to the grant’s bar', async () => {
    installDaemon({ grants: () => [grantRow({ session_id: 'chat-1' })], reads: () => [consent] });
    renderChat();
    expect(await screen.findByTestId('crew-chat-access-bar')).toBeInTheDocument();
    expect(screen.queryByTestId('crew-chat-reads-bar')).toBeNull();
  });

  it('reads again once the person allows the consent on its card', async () => {
    let reads: (typeof consent)[] = [];
    installDaemon({ reads: () => reads });
    renderChat();
    await waitFor(() => expect(grantReads()).toBe(1));
    reads = [consent];
    act(() => {
      window.dispatchEvent(
        new CustomEvent(CREW_READ_CONSENT_EVENT, { detail: { sessionId: 'chat-1' } })
      );
    });
    expect(await screen.findByText(readsCopy.reads('Fixture'))).toBeInTheDocument();
    expect(grantReads()).toBe(2);
  });
});
