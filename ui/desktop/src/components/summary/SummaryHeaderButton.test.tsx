import { useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { SummaryHeaderButton, type SummaryHeaderButtonProps } from './SummaryHeaderButton';
import { summaryCopy } from './copy';

/**
 * The Chat summary's header button, the half of rung 0 a person presses
 * (spec 3.5, "The header button"). `BaseChat` cannot be mounted in jsdom, so
 * the button is its own component and is exercised here in both modes;
 * `BaseChat.summaryRail.test.ts` pins that BaseChat wires it to the rail.
 */

const summary: SummaryHeaderButtonProps['summary'] = {
  toolCalls: '6',
  billedTokens: '211k',
  artifacts: '1',
  codeDelta: { added: 3, removed: 1 },
  hasWorkflow: false,
  onWorkflow: vi.fn(),
  onDiagnostics: vi.fn(),
  todos: { items: [], loading: false, error: false, refresh: vi.fn() },
};

function Harness(props: Partial<SummaryHeaderButtonProps> & { initialOpen?: boolean }) {
  const { initialOpen = false, ...rest } = props;
  const [open, setOpen] = useState(initialOpen);
  return (
    <>
      <SummaryHeaderButton
        mode="popover"
        railShown={false}
        railRendered={false}
        railId="summary-rail"
        onToggleRail={vi.fn()}
        popoverOpen={open}
        onPopoverOpenChange={setOpen}
        progress={null}
        summary={summary}
        {...rest}
      />
      {rest.railRendered && <aside id="summary-rail" aria-label={summaryCopy.title} />}
    </>
  );
}

const button = () => screen.getByRole('button', { name: /^Chat summary/ });

afterEach(cleanup);

describe('the summary button where the rail fits', () => {
  it('shows and hides the docked rail and never opens the popover', async () => {
    const user = userEvent.setup();
    const onToggleRail = vi.fn();
    render(<Harness mode="rail" railShown railRendered onToggleRail={onToggleRail} />);

    expect(button()).toHaveAttribute('data-summary-mode', 'rail');
    expect(button()).toHaveAttribute('aria-expanded', 'true');
    expect(button()).toHaveAttribute('aria-controls', 'summary-rail');
    // A disclosure of the rail, not a popup.
    expect(button()).not.toHaveAttribute('aria-haspopup');

    await user.click(button());
    expect(onToggleRail).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('names nothing it does not control while the rail is folded away', () => {
    render(<Harness mode="rail" />);
    expect(button()).toHaveAttribute('aria-expanded', 'false');
    expect(button()).not.toHaveAttribute('aria-controls');
  });

  it('says what a press will do: hide or show the summary', async () => {
    const user = userEvent.setup();
    const { rerender } = render(<Harness mode="rail" railShown railRendered />);
    await user.tab();
    expect(await screen.findByRole('tooltip')).toHaveTextContent(summaryCopy.hideSummary);

    rerender(<Harness mode="rail" />);
    await waitFor(() =>
      expect(screen.getByRole('tooltip')).toHaveTextContent(summaryCopy.showSummary)
    );
  });
});

describe('the summary button where the rail does not fit', () => {
  it('opens the same summary as a dialog popover anchored to the button', async () => {
    const user = userEvent.setup();
    render(<Harness />);

    expect(button()).toHaveAttribute('data-summary-mode', 'popover');
    expect(button()).toHaveAttribute('aria-haspopup', 'dialog');
    expect(button()).toHaveAttribute('aria-expanded', 'false');
    expect(button()).not.toHaveAttribute('aria-controls');

    await user.click(button());
    const dialog = await screen.findByRole('dialog', { name: summaryCopy.title });
    expect(dialog).toHaveClass('br-summary-popover');
    expect(within(dialog).getByText(summaryCopy.toolCalls)).toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: summaryCopy.diagnostics })).toBeVisible();
    expect(button()).toHaveAttribute('aria-expanded', 'true');
    expect(button()).toHaveAttribute('aria-controls', dialog.id);
  });

  it('closes on a second press instead of dismissing and reopening', async () => {
    const user = userEvent.setup();
    render(<Harness initialOpen />);
    expect(await screen.findByRole('dialog')).toBeInTheDocument();

    await user.click(button());
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(button()).toHaveAttribute('aria-expanded', 'false');
  });

  it('returns focus to the button when Escape closes it', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(button());
    await screen.findByRole('dialog');

    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(button()).toHaveFocus();
  });
});

describe('the progress dot', () => {
  it('marks a running step while the rail is folded away, and puts the count in the name', () => {
    const { container } = render(<Harness mode="rail" progress={{ done: 2, total: 5 }} />);
    expect(
      screen.getByRole('button', { name: 'Chat summary, 2 of 5 to do items complete' })
    ).toBeInTheDocument();
    const dot = container.querySelector('.br-summary-dot');
    expect(dot).not.toBeNull();
    expect(dot).toHaveAttribute('aria-hidden', 'true');
  });

  it('is absent with nothing running', () => {
    const { container } = render(<Harness mode="rail" railShown railRendered />);
    expect(screen.getByRole('button', { name: summaryCopy.title })).toBeInTheDocument();
    expect(container.querySelector('.br-summary-dot')).toBeNull();
  });
});
