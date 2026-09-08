import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ResetPanel from './ResetPanel';

const mocks = vi.hoisted(() => ({
  previewReset: vi.fn(),
  resetAppData: vi.fn(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('../../../api', () => ({
  previewReset: mocks.previewReset,
  resetAppData: mocks.resetAppData,
}));

vi.mock('../../../toasts', () => ({
  toastService: {
    success: mocks.toastSuccess,
    error: mocks.toastError,
  },
}));

describe('ResetPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    mocks.previewReset.mockResolvedValue({
      data: {
        counts: {
          applications: 2,
          knowledgeBases: 3,
          skills: 4,
          extensions: 1,
          schedules: 5,
          workflows: 6,
          conversations: 12,
        },
      },
    });
    mocks.resetAppData.mockResolvedValue({ data: { reset: [], removed: {} } });
  });

  it('shows live counts and requires a deliberate selection for partial reset', async () => {
    render(<ResetPanel />);

    expect(await screen.findByText('2 built')).toBeInTheDocument();
    const resetSelected = screen.getByRole('button', { name: 'Reset selected' });
    expect(resetSelected).toBeDisabled();
    expect(
      screen.queryByText('Delete every app created with Agent Drafter.')
    ).not.toBeInTheDocument();

    const applicationDetails = screen.getByRole('button', {
      name: 'Show details for Built apps',
    });
    expect(applicationDetails).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(applicationDetails);
    expect(applicationDetails).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('Delete every app created with Agent Drafter.')).toBeVisible();

    const workflowDetails = screen.getByRole('button', { name: 'Show details for Workflows' });
    fireEvent.click(workflowDetails);
    expect(applicationDetails).toHaveAttribute('aria-expanded', 'false');
    expect(
      screen.queryByText('Delete every app created with Agent Drafter.')
    ).not.toBeInTheDocument();
    expect(
      screen.getByText('Remove managed workflows and restore the Meditation workflow.')
    ).toBeVisible();

    fireEvent.click(screen.getByRole('checkbox', { name: 'Select Built apps for reset' }));
    expect(resetSelected).toBeEnabled();
    fireEvent.click(resetSelected);

    expect(screen.getByRole('dialog')).toHaveTextContent('Reset selected data?');
    expect(screen.getByRole('dialog')).toHaveTextContent('Built apps');
    expect(screen.getByRole('dialog')).toHaveTextContent(
      'models, provider credentials, theme, and app preferences'
    );
  });

  it('submits only the selected categories and refreshes the preview', async () => {
    const onReset = vi.fn();
    render(<ResetPanel onReset={onReset} />);
    await screen.findByText('12 chats');

    fireEvent.click(
      screen.getByRole('checkbox', { name: 'Select Chat & usage history for reset' })
    );
    fireEvent.click(screen.getByRole('button', { name: 'Reset selected' }));
    fireEvent.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: 'Reset selected' })
    );

    await waitFor(() => {
      expect(mocks.resetAppData).toHaveBeenCalledWith({
        body: { categories: ['history'] },
        throwOnError: true,
      });
    });
    await waitFor(() => expect(onReset).toHaveBeenCalledWith(['history']));
    expect(mocks.previewReset).toHaveBeenCalledTimes(2);
    expect(await screen.findByText(/Reset complete/)).toBeInTheDocument();
  });
});
