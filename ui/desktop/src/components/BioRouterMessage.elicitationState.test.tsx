import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import BioRouterMessage from './BioRouterMessage';
import type { Message } from '../api';
import { createElicitationResponseMessage } from '../types/message';
import { TURN_STOPPED_NOTICE } from './conversation/turnStoppedNotice';

vi.mock('./MarkdownContent', () => ({ default: () => null }));

function request(id = 'elicitation-1'): Message {
  return {
    id: `message-${id}`,
    role: 'assistant',
    created: 1,
    metadata: { userVisible: true, agentVisible: false },
    content: [
      {
        type: 'actionRequired',
        data: {
          actionType: 'elicitation',
          id,
          message: 'Which synthetic cohort?',
          requested_schema: {
            type: 'object',
            properties: { cohort: { type: 'string', title: 'Cohort' } },
            required: ['cohort'],
          },
        },
      },
    ],
  };
}

function stopped(): Message {
  return {
    id: 'stop-message',
    role: 'assistant',
    created: 3,
    metadata: { userVisible: true, agentVisible: true },
    content: [{ type: 'text', text: TURN_STOPPED_NOTICE }],
  };
}

function newTurn(): Message {
  return {
    id: 'next-user-message',
    role: 'user',
    created: 4,
    metadata: { userVisible: true, agentVisible: true },
    content: [{ type: 'text', text: 'Ask another question.' }],
  };
}

function card(message: Message, messages: Message[], submit = vi.fn(async () => undefined)) {
  return (
    <BioRouterMessage
      sessionId="elicitation-state-fixture"
      message={message}
      messages={messages}
      toolCallNotifications={new Map()}
      submitElicitationResponse={submit}
      onRunInTerminal={null}
      onOpenArtifact={vi.fn()}
    />
  );
}

describe('ordinary elicitation state from the stored transcript', () => {
  it('keeps an answered card settled after history reload without exposing its answer', () => {
    const message = request();
    const answer = createElicitationResponseMessage('elicitation-1', {
      cohort: 'synthetic stored answer',
    });
    render(card(message, [message, answer]));

    expect(screen.getByText('Information sent')).toHaveClass('br-transcript-row-label');
    expect(screen.queryByRole('button', { name: 'Submit' })).toBeNull();
    expect(screen.queryByRole('textbox', { name: /^Cohort/ })).toBeNull();
    expect(screen.queryByText('synthetic stored answer')).toBeNull();
  });

  it('settles an already mounted card when its durable answer arrives', () => {
    const message = request();
    const view = render(card(message, [message]));
    expect(screen.getByRole('button', { name: 'Submit' })).toBeEnabled();

    const answer = createElicitationResponseMessage('elicitation-1', { cohort: 'synthetic' });
    view.rerender(card(message, [message, answer]));

    expect(screen.getByText('Information sent')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Submit' })).toBeNull();
  });

  it('removes the pending form after Stop before a late answer can be submitted', () => {
    const message = request();
    const submit = vi.fn(async () => undefined);
    const view = render(card(message, [message], submit));
    fireEvent.change(screen.getByRole('textbox', { name: /^Cohort/ }), {
      target: { value: 'synthetic late answer' },
    });

    view.rerender(card(message, [message, stopped()], submit));

    expect(screen.getByText('Information request canceled')).toHaveClass('br-transcript-row-label');
    expect(screen.queryByRole('button', { name: 'Submit' })).toBeNull();
    expect(screen.queryByRole('textbox', { name: /^Cohort/ })).toBeNull();
    expect(submit).not.toHaveBeenCalled();
  });

  it('preserves an answer acknowledged before the same turn was stopped', () => {
    const message = request();
    const answer = createElicitationResponseMessage('elicitation-1', { cohort: 'synthetic' });
    render(card(message, [message, answer, stopped()]));

    expect(screen.getByText('Information sent')).toBeInTheDocument();
    expect(screen.queryByText('Information request canceled')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Submit' })).toBeNull();
  });

  it('does not apply a different request answer or an earlier turn Stop to a fresh form', () => {
    const old = request('old-elicitation');
    const message = request('fresh-elicitation');
    const oldAnswer = createElicitationResponseMessage('old-elicitation', { cohort: 'synthetic' });
    render(card(message, [old, oldAnswer, stopped(), newTurn(), message]));

    expect(screen.getByRole('button', { name: 'Submit' })).toBeEnabled();
    expect(screen.queryByText('Information sent')).toBeNull();
    expect(screen.queryByText('Information request canceled')).toBeNull();
  });

  it('does not use an answer before the card or a later user turn Stop as its settlement', () => {
    const message = request();
    const earlierAnswer = createElicitationResponseMessage('elicitation-1', {
      cohort: 'synthetic',
    });
    render(card(message, [earlierAnswer, message, newTurn(), stopped()]));

    expect(screen.getByRole('button', { name: 'Submit' })).toBeEnabled();
    expect(screen.queryByText('Information sent')).toBeNull();
    expect(screen.queryByText('Information request canceled')).toBeNull();
  });
});
