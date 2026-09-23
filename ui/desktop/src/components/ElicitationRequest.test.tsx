import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import ElicitationRequest from './ElicitationRequest';

vi.mock('./ui/JsonSchemaForm', () => ({
  default: ({ onSubmit, submitLabel }: { onSubmit: (data: Record<string, unknown>) => void; submitLabel?: string }) => (
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
    expect(screen.queryByText('Information submitted')).toBeNull();
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
    await waitFor(() => expect(screen.getByText('Information submitted')).toBeInTheDocument());
    expect(onSubmit).toHaveBeenCalledWith('request-1', { cohort: 'synthetic' });
  });
});
