import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import ElicitationRequest from './ElicitationRequest';
import { APPROVAL_COPY } from './toolCallCopy';
import { BROWSER_SURFACE_MARKER } from '../utils/surface';

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

vi.mock('./ui/JsonSchemaForm', () => ({
  default: ({
    onSubmit,
    onCancel,
    cancelLabel,
    submitLabel,
  }: {
    onSubmit: (data: Record<string, unknown>) => void;
    onCancel?: () => void;
    cancelLabel?: string;
    submitLabel?: string;
  }) => (
    <>
      <button type="button" onClick={() => onSubmit({ cohort: 'synthetic' })}>
        {submitLabel ?? 'Submit'}
      </button>
      {onCancel ? (
        <button type="button" onClick={onCancel}>
          {cancelLabel ?? 'Cancel'}
        </button>
      ) : null}
    </>
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

  it('answers Cancel with a null response and settles as canceled', async () => {
    const onSubmit = vi.fn(async () => undefined);
    render(
      <ElicitationRequest
        actionRequiredContent={action}
        isCancelledMessage={false}
        isClicked={false}
        onSubmit={onSubmit}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() =>
      expect(screen.getByText('Information request canceled')).toBeInTheDocument()
    );
    expect(onSubmit).toHaveBeenCalledWith('request-1', null);
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
    expect(card).toHaveClass('br-action-card');
    // A filled card (R-27): its fields step back to the page ground.
    expect(card).toHaveClass('br-fill-surface');
    expect(card.className).not.toContain('rounded-2xl');
    expect(screen.getByText('Which cohort?')).toHaveClass('text-label');
  });

  // Owner, 2026-11-13: a server's paragraph of instructions does not become a wall of title.
  // The first sentence is the title; a short rest is the info icon's, a long one folds.
  it('titles a long request with its first sentence and folds the rest', () => {
    const first = 'Choose the cohort this analysis should run on before anything is computed.';
    const short = 'Only cohorts you can read are listed.';
    const { unmount } = render(
      <ElicitationRequest
        actionRequiredContent={{
          ...action,
          data: { ...action.data, message: `${first} ${short}` },
        }}
        isCancelledMessage={false}
        isClicked={false}
        onSubmit={vi.fn(async () => undefined)}
      />
    );
    expect(screen.getByText(first)).toHaveClass('text-label');
    expect(screen.getByRole('button', { name: 'About this request' })).toHaveAccessibleDescription(
      short
    );
    unmount();

    const long =
      'Only cohorts you can read are listed. Each one is named by its study. Pick one and press Submit.';
    render(
      <ElicitationRequest
        actionRequiredContent={{ ...action, data: { ...action.data, message: `${first} ${long}` } }}
        isCancelledMessage={false}
        isClicked={false}
        onSubmit={vi.fn(async () => undefined)}
      />
    );
    expect(screen.getByText(first)).toHaveClass('text-label');
    expect(screen.queryByText(long)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Details' }));
    expect(screen.getByText(long)).toBeInTheDocument();
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

  it('honors a settled answer received after the form mounted', () => {
    const onSubmit = vi.fn(async () => undefined);
    const view = render(
      <ElicitationRequest
        actionRequiredContent={action}
        isCancelledMessage={false}
        isClicked={false}
        onSubmit={onSubmit}
      />
    );
    expect(screen.getByRole('button', { name: 'Submit' })).toBeEnabled();

    view.rerender(
      <ElicitationRequest
        actionRequiredContent={action}
        isCancelledMessage={false}
        isClicked
        onSubmit={onSubmit}
      />
    );

    expect(screen.getByText('Information sent')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Submit' })).toBeNull();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('keeps a request canceled if an already in-flight delivery finishes after Stop', async () => {
    let finish!: () => void;
    const onSubmit = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        })
    );
    const view = render(
      <ElicitationRequest
        actionRequiredContent={action}
        isCancelledMessage={false}
        isClicked={false}
        onSubmit={onSubmit}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Submit' }));
    view.rerender(
      <ElicitationRequest
        actionRequiredContent={action}
        isCancelledMessage
        isClicked={false}
        onSubmit={onSubmit}
      />
    );
    finish();

    await waitFor(() =>
      expect(screen.getByText('Information request canceled')).toBeInTheDocument()
    );
    expect(screen.queryByText('Information sent')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Submit' })).toBeNull();
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it('keeps an acknowledged answer sent when Stop arrives before its durable transcript row', async () => {
    const onSubmit = vi.fn(async () => undefined);
    const view = render(
      <ElicitationRequest
        actionRequiredContent={action}
        isCancelledMessage={false}
        isClicked={false}
        onSubmit={onSubmit}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Submit' }));
    await waitFor(() => expect(screen.getByText('Information sent')).toBeInTheDocument());

    view.rerender(
      <ElicitationRequest
        actionRequiredContent={action}
        isCancelledMessage
        isClicked={false}
        onSubmit={onSubmit}
      />
    );

    expect(screen.getByText('Information sent')).toBeInTheDocument();
    expect(screen.queryByText('Information request canceled')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Submit' })).toBeNull();
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });
});

describe('ordinary elicitation surface boundaries', () => {
  it('explains the desktop requirement before offering an unusable browser form', () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    const onSubmit = vi.fn(async () => undefined);
    render(
      <ElicitationRequest
        actionRequiredContent={action}
        isCancelledMessage={false}
        isClicked={false}
        onSubmit={onSubmit}
      />
    );

    expect(screen.getByRole('status')).toHaveTextContent(APPROVAL_COPY.browserCannotApprove);
    expect(
      screen.getByRole('button', { name: 'About ' + APPROVAL_COPY.browserCannotApproveName })
    ).toHaveAccessibleDescription(APPROVAL_COPY.browserCannotApproveWhy);
    expect(screen.queryByRole('button', { name: 'Submit' })).toBeNull();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it.each([
    { isClicked: true, isCancelledMessage: false, label: 'Information sent' },
    { isClicked: false, isCancelledMessage: true, label: 'Information request canceled' },
  ])('keeps the settled $label row in the browser', ({ isClicked, isCancelledMessage, label }) => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    render(
      <ElicitationRequest
        actionRequiredContent={action}
        isCancelledMessage={isCancelledMessage}
        isClicked={isClicked}
        onSubmit={vi.fn(async () => undefined)}
      />
    );

    expect(screen.getByText(label)).toHaveClass('br-transcript-row-label');
    expect(screen.queryByText(APPROVAL_COPY.browserCannotApprove)).toBeNull();
    expect(screen.queryByRole('button', { name: 'Submit' })).toBeNull();
  });

  it('shows the generated client error body without marking an answer sent', async () => {
    const message = 'Verified human authority is required to answer this request';
    const onSubmit = vi.fn().mockRejectedValue({ error: message });
    render(
      <ElicitationRequest
        actionRequiredContent={action}
        isCancelledMessage={false}
        isClicked={false}
        onSubmit={onSubmit}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Submit' }));
    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent(message));
    expect(screen.getByRole('alert')).not.toHaveTextContent('[object Object]');
    expect(screen.getByRole('button', { name: 'Submit' })).toBeInTheDocument();
    expect(screen.queryByText('Information sent')).toBeNull();
  });
});
