import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import ElicitationRequest from './ElicitationRequest';

vi.mock('./ui/JsonSchemaForm', () => ({
  default: ({
    onSubmit,
    submitLabel,
  }: {
    onSubmit: (data: Record<string, unknown>) => void;
    submitLabel?: string;
  }) => (
    <button type="button" onClick={() => onSubmit({ cohort: 'synthetic' })}>
      {submitLabel ?? 'Submit'}
    </button>
  ),
}));

const action = {
  type: 'actionRequired' as const,
  data: {
    actionType: 'elicitation' as const,
    id: 'request-1',
    message: 'Which cohort?',
    requested_schema: { type: 'object' },
  },
};

describe('ElicitationRequest', () => {
  it('keeps the form visible and reports a failed delivery', async () => {
    const onSubmit = vi.fn(async () => {
      throw new Error('request expired');
    });
    render(
      <ElicitationRequest
        actionRequiredContent={action}
        isCancelledMessage={false}
        isClicked={false}
        onSubmit={onSubmit}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Submit' }));
    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('request expired'));
    expect(screen.getByRole('button', { name: 'Submit' })).toBeInTheDocument();
    expect(screen.queryByText('Information sent')).toBeNull();
  });

  it('marks the request submitted only after delivery succeeds', async () => {
    const onSubmit = vi.fn(async () => undefined);
    render(
      <ElicitationRequest
        actionRequiredContent={action}
        isCancelledMessage={false}
        isClicked={false}
        onSubmit={onSubmit}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Submit' }));
    // The answered card collapses to a transcript row.
    await waitFor(() =>
      expect(screen.getByText('Information sent')).toHaveClass('br-transcript-row-label')
    );
    expect(onSubmit).toHaveBeenCalledWith('request-1', { cohort: 'synthetic' });
  });

  it('asks in the one card recipe with the question as its title', () => {
    const { container } = render(
      <ElicitationRequest
        actionRequiredContent={action}
        isCancelledMessage={false}
        isClicked={false}
        onSubmit={vi.fn(async () => undefined)}
      />
    );
    const card = container.querySelector('.biorouter-message-content')!;
    expect(card.className).toContain('rounded-container');
    expect(card.className).not.toContain('rounded-2xl');
    expect(screen.getByText('Which cohort?')).toHaveClass('text-label');
  });

  it('shows a canceled request as a quiet row', () => {
    render(
      <ElicitationRequest
        actionRequiredContent={action}
        isCancelledMessage
        isClicked={false}
        onSubmit={vi.fn(async () => undefined)}
      />
    );
    expect(screen.getByText('Information request canceled')).toHaveClass('br-transcript-row-label');
  });
});
