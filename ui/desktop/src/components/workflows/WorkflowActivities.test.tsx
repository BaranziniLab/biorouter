import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import WorkflowActivities from './WorkflowActivities';

describe('WorkflowActivities (a workflow chat’s starters)', () => {
  it('draws each starter as a secondary button the keyboard reaches', async () => {
    const user = userEvent.setup();
    const append = vi.fn();
    render(
      <WorkflowActivities
        append={append}
        activities={['Summarize {{topic}}', 'List sources']}
        parameterValues={{ topic: 'MS' }}
      />
    );

    const first = screen.getByRole('button', { name: 'Summarize MS' });
    await user.tab();
    expect(first).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(append).toHaveBeenCalledWith('Summarize MS');
    expect(screen.getByRole('button', { name: 'List sources' })).toBeInTheDocument();
  });

  it('cuts a long starter with an ellipsis and never uses title=', () => {
    const long = 'x'.repeat(80);
    render(<WorkflowActivities append={vi.fn()} activities={[long]} />);
    const button = screen.getByRole('button');
    expect(button).toHaveTextContent(`${'x'.repeat(60)}…`);
    expect(button).not.toHaveAttribute('title');
  });

  it('shows the workflow’s message as prose, outside the starter row', () => {
    render(
      <WorkflowActivities
        append={vi.fn()}
        activities={['message: Welcome to the review', 'Start']}
      />
    );
    expect(screen.getByText('Welcome to the review')).toBeInTheDocument();
    expect(screen.getAllByRole('button')).toHaveLength(1);
  });

  it('renders nothing without activities', () => {
    const { container } = render(<WorkflowActivities append={vi.fn()} activities={null} />);
    expect(container).toBeEmptyDOMElement();
  });
});
