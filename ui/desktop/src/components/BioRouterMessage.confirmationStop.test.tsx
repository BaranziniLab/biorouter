import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import BioRouterMessage from './BioRouterMessage';
import type { Message } from '../api';
import { TURN_STOPPED_NOTICE } from './conversation/turnStoppedNotice';

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

function confirmation(): Message {
  return {
    id: `confirmation-message-${nextId++}`,
    role: 'assistant',
    created: 1,
    metadata: { userVisible: true, agentVisible: false },
    content: [
      {
        type: 'actionRequired',
        data: {
          actionType: 'toolConfirmation',
          id: `confirmation-${nextId++}`,
          toolName: 'developer__shell',
          arguments: { command: 'printf fixture' },
          prompt: null,
        },
      },
    ],
  } as Message;
}

function stopped(): Message {
  return {
    id: `stop-${nextId++}`,
    role: 'assistant',
    created: 2,
    metadata: { userVisible: true, agentVisible: false },
    content: [
      { type: 'systemNotification', notificationType: 'inlineMessage', msg: TURN_STOPPED_NOTICE },
    ],
  };
}

function userMessage(visible: boolean): Message {
  return {
    id: `user-${nextId++}`,
    role: 'user',
    created: 3,
    metadata: { userVisible: visible, agentVisible: true },
    content: [{ type: 'text', text: visible ? 'A new request' : 'Internal tool instructions' }],
  };
}

function card(message: Message, messages: Message[]) {
  return (
    <BioRouterMessage
      sessionId="stopped-approval-fixture"
      message={message}
      messages={messages}
      toolCallNotifications={new Map()}
      onRunInTerminal={null}
      onOpenArtifact={vi.fn()}
    />
  );
}

beforeEach(() => {
  mocks.confirmToolAction.mockReset().mockResolvedValue({ data: { status: 'delivered' } });
});

describe('approval cards after Stop', () => {
  it('removes pending controls when the daemon stop notice arrives', () => {
    const message = confirmation();
    const view = render(card(message, [message]));
    expect(screen.getByRole('button', { name: 'Allow once' })).toBeEnabled();
    view.rerender(card(message, [message, userMessage(false), stopped()]));
    expect(screen.getByText('Approval canceled')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Allow once' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Deny' })).toBeNull();
    expect(mocks.confirmToolAction).not.toHaveBeenCalled();
  });

  it('keeps a fresh approval after an earlier turn was stopped', () => {
    const old = confirmation();
    const fresh = confirmation();
    render(card(fresh, [old, stopped(), userMessage(true), fresh]));
    expect(screen.getByRole('button', { name: 'Allow once' })).toBeEnabled();
    expect(screen.queryByText('Approval canceled')).toBeNull();
  });

  it('does not apply a later user turn’s Stop to the earlier card', () => {
    const message = confirmation();
    render(card(message, [message, userMessage(true), stopped()]));
    expect(screen.getByRole('button', { name: 'Allow once' })).toBeEnabled();
  });

  it('requires the typed daemon notice rather than model prose', () => {
    const message = confirmation();
    const prose: Message = { ...stopped(), content: [{ type: 'text', text: TURN_STOPPED_NOTICE }] };
    render(card(message, [message, prose]));
    expect(screen.getByRole('button', { name: 'Allow once' })).toBeEnabled();
  });

  it('preserves an approval acknowledged before the turn stopped', async () => {
    const message = confirmation();
    const view = render(card(message, [message]));
    fireEvent.click(screen.getByRole('button', { name: 'Allow once' }));
    expect(await screen.findByText('Shell is allowed once')).toBeInTheDocument();
    view.rerender(card(message, [message, stopped()]));
    expect(screen.getByText('Shell is allowed once')).toBeInTheDocument();
    expect(screen.queryByText('Approval canceled')).toBeNull();
    expect(mocks.confirmToolAction).toHaveBeenCalledTimes(1);
  });
});
