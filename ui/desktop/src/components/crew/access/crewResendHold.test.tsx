import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Message } from '../../../api';
import UserMessage from '../../UserMessage';
import { useChatCrewAccess } from './chatCrewAccess';
import { ChatCrewAccessBar } from './ChatCrewAccessBar';
import { accessCopy } from './copy';
import { useCrewResendHold } from './crewResendHold';
import { connection, grantRow } from './testing';
import { forgetUnconfirmedRevocations } from './useCrewGrants';

/**
 * Final acceptance F1: in a chat whose Crew access was revoked, **Edit in place** bypassed the
 * composer's hold. The chat stream truncated the stored conversation at the edited message
 * (20 and then 24 rows, measured) and only then started a turn the daemon refused. Every path that
 * sends without the composer now asks the hold first, and a refused edit changes nothing — not the
 * store, not the editor.
 */

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn(), toastWarning: vi.fn() }));
vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});
vi.mock('../../../toasts', async () => {
  const actual = await vi.importActual<typeof import('../../../toasts')>('../../../toasts');
  return { ...actual, toastWarning: mocks.toastWarning };
});

let grants: Record<string, unknown>[];

const message: Message = {
  id: 'message-1',
  role: 'user',
  created: 1,
  content: [{ type: 'text', text: 'summarise #data' }],
  metadata: { userVisible: true, agentVisible: true },
};

/**
 * What the chat stream's own edit does first: `ChatStreamController.onMessageUpdate` calls
 * `editMessage`, which truncates the stored session. It must never be reached in a held chat.
 */
const streamEdit = vi.fn(
  async (_id: string, _content: string, _editType?: 'diverge' | 'edit'): Promise<void> => undefined
);

/** BaseChat's wiring, as it is: the lookup, the bar, the hold, and the transcript's edit. */
function Chat() {
  const access = useChatCrewAccess('chat-1');
  const refuse = useCrewResendHold(access);
  const onMessageUpdate = (id: string, content: string, editType?: 'diverge' | 'edit') =>
    refuse() ? false : streamEdit(id, content, editType);
  return (
    <div>
      <ChatCrewAccessBar access={access} chatTitle="Slack data summary request" />
      <p data-testid="blocked">{String(access.blocksComposer)}</p>
      <UserMessage message={message} onMessageUpdate={onMessageUpdate} />
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

function edit(to: string, save: 'Edit message in place' | 'Diverge with the edited message') {
  fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));
  fireEvent.change(screen.getByRole('textbox', { name: 'Edit message content' }), {
    target: { value: to },
  });
  fireEvent.click(screen.getByRole('button', { name: save }));
}

beforeEach(() => {
  vi.clearAllMocks();
  forgetUnconfirmedRevocations();
  Object.assign(window, { electron: { logInfo: vi.fn() } });
  grants = [grantRow({ session_id: 'chat-1' })];
  mocks.crewHttp.mockImplementation(async (requested: string, method = 'GET') => {
    if (requested === '/connections') return { connections: [connection] };
    if (requested === '/connections/conn-1/grants' && method === 'GET') return { grants };
    return {};
  });
});

describe('the Crew hold covers edits and every other re-send (F1)', () => {
  it.each(['Edit message in place', 'Diverge with the edited message'] as const)(
    'refuses “%s” in a revoked chat before anything is cut, and keeps the words',
    async (save) => {
      grants = [grantRow({ session_id: 'chat-1', expired: true })];
      renderChat();
      await waitFor(() => expect(screen.getByTestId('blocked')).toHaveTextContent('true'));

      edit('summarise #data again', save);

      // Nothing reached the chat stream, so nothing was truncated and no turn started.
      expect(streamEdit).not.toHaveBeenCalled();
      // The person is told why, in the hold's own words.
      expect(mocks.toastWarning).toHaveBeenCalledWith(
        expect.objectContaining({ title: accessCopy.chatBlockedSendTitle })
      );
      // The editor stays open with what they wrote, and the message is not marked edited.
      expect(screen.getByRole('textbox', { name: 'Edit message content' })).toHaveValue(
        'summarise #data again'
      );
      expect(screen.queryByText('Edited')).toBeNull();
    }
  );

  it('lets the same edit through while the chat’s access is active', async () => {
    renderChat();
    await waitFor(() =>
      expect(screen.getByRole('button', { name: accessCopy.revokeButton })).toBeInTheDocument()
    );
    edit('summarise #data again', 'Edit message in place');
    expect(streamEdit).toHaveBeenCalledWith('message-1', 'summarise #data again', 'edit');
    expect(mocks.toastWarning).not.toHaveBeenCalled();
    expect(screen.queryByRole('textbox', { name: 'Edit message content' })).toBeNull();
  });
});

/**
 * BaseChat cannot be mounted in jsdom (see `BaseChat.privacy.test.tsx`), so its half is pinned at
 * the source: every path that sends without the composer goes through the hold, and none reaches
 * the chat stream's own function directly.
 */
describe('BaseChat routes every re-send through the Crew hold (F1)', () => {
  const source = readFileSync(
    path.join(process.cwd(), 'src', 'components', 'BaseChat.tsx'),
    'utf8'
  );

  it('hands the transcript, the turn error, the composer and the workflow only held senders', () => {
    expect(source).toMatch(/onMessageUpdate=\{heldMessageUpdate\}/);
    expect(source).toMatch(/<ChatTurnError error=\{turnError\} onRetry=\{heldRetryTurn\} \/>/);
    expect(source).toMatch(/onSteer=\{heldSteer\}/);
    expect(source).toMatch(/append=\{\(text: string\) => heldSubmit\(text\)\}/);
    expect(source).toMatch(/submitAndReturnToBottom\(\{ sessionId, submit: heldSubmit \}/);
    expect(source).toMatch(/submit: heldSubmit,/);
    // The unguarded senders are never handed out.
    expect(source).not.toMatch(/onMessageUpdate=\{onMessageUpdate\}/);
    expect(source).not.toMatch(/onRetry=\{retryTurn\}/);
    expect(source).not.toMatch(/onSteer=\{steer\}/);
    expect(source).not.toMatch(/submit: handleSubmit\b/);
  });

  it('asks the hold before an artifact repair starts a turn of its own accord', () => {
    const repair = /const submitArtifactRepairMessage = useCallback\([\s\S]*?\n {2}\);/.exec(
      source
    );
    expect(repair?.[0]).toMatch(/refuseWhileCrewHeld\(\{ quiet: true \}\)\) return false;/);
  });
});
