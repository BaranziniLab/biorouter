import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { WorkflowWarningModal } from './WorkflowWarningModal';

vi.mock('../MarkdownContent', () => ({
  default: ({ content }: { content: string }) => <div data-testid="markdown">{content}</div>,
}));

const details = {
  title: 'Cohort summary',
  description: 'Summarizes a cohort.',
  instructions: 'Read the table.',
};

describe('WorkflowWarningModal', () => {
  it('is the shared dialog: modal, on the dialog motion, with no close button', () => {
    render(
      <WorkflowWarningModal
        isOpen
        onConfirm={() => {}}
        onCancel={() => {}}
        workflowDetails={details}
      />
    );
    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveAttribute('aria-modal', 'true');
    expect(dialog).toHaveClass('br-dialog-motion');
    // The old hand-rolled content used Tailwind's 95% zoom and a 200ms duration of its own.
    expect(dialog.className).not.toMatch(/zoom-(in|out)-95|duration-200/);
    expect(screen.queryByRole('button', { name: 'Close' })).toBeNull();
  });

  it('says its title in sentence case, without an emoji', () => {
    const { rerender } = render(
      <WorkflowWarningModal
        isOpen
        onConfirm={() => {}}
        onCancel={() => {}}
        workflowDetails={details}
      />
    );
    expect(screen.getByRole('heading', { name: 'New workflow warning' })).toBeInTheDocument();
    rerender(
      <WorkflowWarningModal
        isOpen
        hasSecurityWarnings
        onConfirm={() => {}}
        onCancel={() => {}}
        workflowDetails={details}
      />
    );
    expect(screen.getByRole('heading', { name: 'Security warning' })).toBeInTheDocument();
    expect(screen.getByText(/hidden characters that will be ignored/)).toBeInTheDocument();
  });

  it('makes the person answer: Escape does nothing, the two buttons decide', async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(
      <WorkflowWarningModal
        isOpen
        onConfirm={onConfirm}
        onCancel={onCancel}
        workflowDetails={details}
      />
    );
    await user.keyboard('{Escape}');
    expect(onCancel).not.toHaveBeenCalled();
    expect(screen.getByRole('dialog')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Trust and execute' }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('shows what is being trusted', () => {
    render(
      <WorkflowWarningModal
        isOpen
        onConfirm={() => {}}
        onCancel={() => {}}
        workflowDetails={details}
      />
    );
    expect(screen.getByText('Cohort summary')).toBeInTheDocument();
    expect(screen.getByText('Summarizes a cohort.')).toBeInTheDocument();
    expect(screen.getByTestId('markdown')).toHaveTextContent('Read the table.');
  });
});
