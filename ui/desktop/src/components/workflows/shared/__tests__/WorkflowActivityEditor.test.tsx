import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

import WorkflowActivityEditor from '../../WorkflowActivityEditor';
import { workflowFormCopy } from '../../copy';

const copy = workflowFormCopy.activities;

describe('WorkflowActivityEditor', () => {
  const mockOnChange = vi.fn();
  const mockOnBlur = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('Basic Rendering', () => {
    it('renders without crashing', () => {
      render(<WorkflowActivityEditor activities={[]} setActivities={mockOnChange} />);
      expect(screen.getByText('Activities')).toBeInTheDocument();
    });

    it('displays the activities label', () => {
      render(<WorkflowActivityEditor activities={[]} setActivities={mockOnChange} />);
      expect(screen.getByText('Activities')).toBeInTheDocument();
    });

    it('keeps its help behind InfoTips, not in paragraphs', () => {
      render(<WorkflowActivityEditor activities={[]} setActivities={mockOnChange} />);
      expect(
        screen.getByRole('button', { name: `About ${copy.title}` })
      ).toHaveAccessibleDescription(copy.info);
      expect(screen.getByRole('textbox', { name: copy.message })).toHaveAccessibleDescription(
        copy.messageInfo
      );
      expect(
        screen.getByRole('button', { name: `About ${copy.buttons}` })
      ).toHaveAccessibleDescription(copy.buttonsInfo);
    });
  });

  describe('Empty State', () => {
    it('shows message input when no activities', () => {
      render(<WorkflowActivityEditor activities={[]} setActivities={mockOnChange} />);
      expect(screen.getByRole('textbox', { name: copy.message })).toHaveAttribute(
        'placeholder',
        copy.messagePlaceholder
      );
    });
  });

  describe('With Activities', () => {
    it('displays existing activities as visual boxes', () => {
      const activities = ['message: Hello World', 'button: Click me', 'action: Do something'];
      render(<WorkflowActivityEditor activities={activities} setActivities={mockOnChange} />);

      const messageTextarea = screen.getByRole('textbox', { name: copy.message });
      expect(messageTextarea).toHaveValue(' Hello World');

      expect(screen.getByText('button: Click me')).toBeInTheDocument();
      expect(screen.getByText('action: Do something')).toBeInTheDocument();

      const removeButtons = screen.getAllByRole('button', { name: /^Remove / });
      expect(removeButtons).toHaveLength(2);
      expect(removeButtons[0]).toHaveAccessibleName(copy.remove('button: Click me'));
    });

    it('truncates long activity text in boxes', () => {
      const longActivity = 'button: ' + 'a'.repeat(150);
      const activities = [longActivity];
      render(<WorkflowActivityEditor activities={activities} setActivities={mockOnChange} />);

      expect(screen.getByText(/button: a+…/)).toBeInTheDocument();

      // The whole label is the row's tooltip.
      const row = screen.getByText(/button: a+…/).closest('li');
      expect(row).toHaveAttribute('title', longActivity);
    });

    it('handles empty activities array', () => {
      render(<WorkflowActivityEditor activities={[]} setActivities={mockOnChange} />);
      expect(screen.getByText('Activities')).toBeInTheDocument();

      expect(screen.queryByRole('button', { name: /^Remove / })).not.toBeInTheDocument();
    });

    it('allows removing activities via remove buttons', async () => {
      const user = userEvent.setup();
      const activities = ['button: Click me', 'action: Do something'];
      render(<WorkflowActivityEditor activities={activities} setActivities={mockOnChange} />);

      const removeButtons = screen.getAllByRole('button', { name: /^Remove / });
      await user.click(removeButtons[0]);

      expect(mockOnChange).toHaveBeenCalledWith(['action: Do something']);
    });
  });

  describe('User Interactions', () => {
    it('adds a starter button on Enter, on the shared field skin', async () => {
      const user = userEvent.setup();
      render(<WorkflowActivityEditor activities={[]} setActivities={mockOnChange} />);

      const input = screen.getByRole('textbox', { name: copy.buttons });
      expect(input).toHaveClass('br-field');
      await user.type(input, 'Summarize{Enter}');

      expect(mockOnChange).toHaveBeenCalledWith(['Summarize']);
    });

    it('allows typing in message field', async () => {
      const user = userEvent.setup();
      render(<WorkflowActivityEditor activities={[]} setActivities={mockOnChange} />);

      const messageInput = screen.getByRole('textbox', { name: copy.message });
      await user.type(messageInput, 'Test message');

      expect(messageInput).toHaveValue('Test message');
    });

    it('calls onBlur when provided', async () => {
      const user = userEvent.setup();
      render(
        <WorkflowActivityEditor activities={[]} setActivities={mockOnChange} onBlur={mockOnBlur} />
      );

      const messageInput = screen.getByRole('textbox', { name: copy.message });
      await user.click(messageInput);
      await user.tab();

      expect(mockOnBlur).toHaveBeenCalled();
    });
  });
});
