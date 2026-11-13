/**
 * An approval card whose answer window closed unanswered.
 *
 * The bug (found live by the decisions lane): a folder card kept "Allow / Allow
 * for this session / Deny" after the daemon's 300 s window closed. The parked
 * call had already given up, so a late click answered nothing
 * (`status: unknown`) and the card collapsed to "no longer available", which
 * made "Allow for this session" look broken. The daemon now follows the card
 * with an inline notice naming it (`expiredActionId`), and the card shows
 * "Expired: ask again" with its buttons gone: when the notice arrives, when a
 * transcript is loaded that already holds both, and when a late click is
 * answered `unknown`.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import BioRouterMessage from './BioRouterMessage';
import ProgressiveMessageList from './ProgressiveMessageList';
import type { Message } from '../api';
import { APPROVAL_COPY } from './toolCallCopy';

const mocks = vi.hoisted(() => ({
  confirmToolAction: vi.fn(),
}));

vi.mock('../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  confirmToolAction: mocks.confirmToolAction,
}));
vi.mock('../utils/userAction', () => ({
  userActionHeaders: vi.fn().mockResolvedValue({ 'X-User-Action': 'fixture-user-proof' }),
}));
vi.mock('./settings/permission/PermissionModal', () => ({ default: () => null }));
vi.mock('./MarkdownContent', () => ({ default: () => null }));

let nextId = 0;

/** The folder card the bug was found on. */
function folderCard(): Message {
  return {
    id: `card-message-${nextId++}`,
    role: 'assistant',
    created: 1,
    metadata: { userVisible: true, agentVisible: false },
    content: [
      {
        type: 'actionRequired',
        data: {
          actionType: 'toolConfirmation',
          id: `card-${nextId++}`,
          toolName: 'filesystem__folder_access',
          arguments: {},
          prompt: 'External folder access',
          preview: {
            kind: 'folderAccess',
            folder: '/synthetic/results',
            access: 'read-only',
            files: ['/synthetic/results/completed.csv'],
            descendants: true,
            onceDescendants: false,
            actionId: 'synthetic-action',
          },
        },
      },
    ],
  } as Message;
}

function cardId(message: Message): string {
  const content = message.content[0];
  if (content.type !== 'actionRequired' || !('id' in content.data)) throw new Error('not a card');
  return content.data.id;
}

/** The daemon's notice that the card's window closed (`card_expiry_notice`). */
function expiry(id: string): Message {
  return {
    role: 'assistant',
    created: 2,
    metadata: { userVisible: true, agentVisible: false },
    content: [
      {
        type: 'systemNotification',
        notificationType: 'inlineMessage',
        msg: 'The folder access request for /synthetic/results expired before it was answered. The tool was not run. Ask again to retry.',
        expiredActionId: id,
      },
    ],
  } as Message;
}

function card(message: Message, messages: Message[]) {
  return (
    <BioRouterMessage
      sessionId="expired-approval-fixture"
      message={message}
      messages={messages}
      toolCallNotifications={new Map()}
      onRunInTerminal={null}
      onOpenArtifact={vi.fn()}
    />
  );
}

function expectNoDecision() {
  for (const name of ['Allow once', 'Allow for this session', 'Deny', 'Cancel operation']) {
    expect(screen.queryByRole('button', { name })).toBeNull();
  }
}

beforeEach(() => {
  mocks.confirmToolAction.mockReset().mockResolvedValue({ data: { status: 'delivered' } });
});

describe('an approval card whose window closed', () => {
  it('drops its buttons and says Expired when the daemon reports the expiry', () => {
    const message = folderCard();
    const view = render(card(message, [message]));
    expect(screen.getByRole('button', { name: 'Allow for this session' })).toBeEnabled();

    view.rerender(card(message, [message, expiry(cardId(message))]));

    expect(screen.getByTestId('tool-approval-expired')).toHaveTextContent(APPROVAL_COPY.expired);
    expect(APPROVAL_COPY.expired).toBe('Expired: ask again');
    expectNoDecision();
    expect(screen.queryByText(/no longer available/)).toBeNull();
    expect(mocks.confirmToolAction).not.toHaveBeenCalled();
  });

  it('is drawn expired when a reloaded transcript already holds the card and its expiry', () => {
    const message = folderCard();
    render(card(message, [message, expiry(cardId(message))]));
    expect(screen.getByText(APPROVAL_COPY.expired)).toBeInTheDocument();
    expectNoDecision();
  });

  it('is calm: muted ink, never the danger or warning tone', () => {
    const message = folderCard();
    render(card(message, [message, expiry(cardId(message))]));
    const row = screen.getByTestId('tool-approval-expired');
    expect(row.innerHTML).not.toMatch(/danger|warning/);
    expect(row.querySelector('[role="alert"]')).toBeNull();
  });

  it('is untouched by another card’s expiry', () => {
    const message = folderCard();
    const other = folderCard();
    render(card(message, [message, other, expiry(cardId(other))]));
    expect(screen.getByRole('button', { name: 'Allow for this session' })).toBeEnabled();
    expect(screen.queryByText(APPROVAL_COPY.expired)).toBeNull();
  });

  it('says Expired, not "no longer available", when a late click finds nothing waiting', async () => {
    mocks.confirmToolAction.mockResolvedValueOnce({ data: { status: 'unknown' } });
    const message = folderCard();
    render(card(message, [message]));
    fireEvent.click(screen.getByRole('button', { name: 'Allow for this session' }));
    expect(await screen.findByText(APPROVAL_COPY.expired)).toBeInTheDocument();
    expectNoDecision();
    expect(screen.queryByText(/no longer available|is always allowed/)).toBeNull();
  });
});

describe('the expiry notice in the transcript', () => {
  function list(messages: Message[]) {
    return render(
      <ProgressiveMessageList
        messages={messages}
        chat={{ sessionId: 'expired-approval-fixture' }}
        toolCallNotifications={new Map()}
        isUserMessage={(m: Message) => m.role !== 'assistant'}
        onRunInTerminal={null}
        onOpenArtifact={vi.fn()}
      />
    );
  }

  it('is drawn on its card rather than as a second line', () => {
    const message = folderCard();
    list([message, expiry(cardId(message))]);
    expect(screen.getByText(APPROVAL_COPY.expired)).toBeInTheDocument();
    expect(screen.queryByText(/expired before it was answered/)).toBeNull();
  });

  it('keeps its own line when the card is not in the transcript', () => {
    list([expiry('a-card-this-view-never-saw')]);
    expect(screen.getByText(/expired before it was answered/)).toBeInTheDocument();
  });
});
