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
        railStill
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

/**
 * Vision V2-R2-6: dragging the window across the 1344px seam flipped the
 * pressed fill twice, and the Button's colour transitions played while the rest
 * of the chat snapped. jsdom runs no transition, so what is asserted here is the
 * marker the stylesheet keys on (`[data-summary-mode][data-still]`, pinned in
 * `styles/summaryRail.test.ts`, measured in `summaryRailGeometry.browser.test.ts`).
 */
describe('the pressed fill follows the person, not geometry', () => {
  /** A rail the button really toggles, as `useSummaryRail` does. */
  function RailHarness({ fits = true, onToggle }: { fits?: boolean; onToggle?: () => void }) {
    const [shown, setShown] = useState(true);
    const [still, setStill] = useState(true);
    const [open, setOpen] = useState(false);
    return (
      <SummaryHeaderButton
        mode={fits ? 'rail' : 'popover'}
        railShown={fits && shown}
        railRendered={fits && shown}
        railId="summary-rail"
        railStill={still}
        onToggleRail={() => {
          onToggle?.();
          // The hook marks the entrance it plays on open, and only then.
          setStill(shown);
          setShown(!shown);
        }}
        popoverOpen={open}
        onPopoverOpenChange={setOpen}
        progress={null}
        summary={summary}
      />
    );
  }

  it('snaps when a resize hides the rail at the seam and when it brings it back', async () => {
    const { rerender } = render(<RailHarness />);
    expect(button()).toHaveAttribute('aria-expanded', 'true');
    expect(button()).not.toHaveAttribute('data-still');

    rerender(<RailHarness fits={false} />);
    expect(button()).toHaveAttribute('aria-expanded', 'false');
    expect(button()).toHaveAttribute('data-still', '');
    // Cleared once a frame has drawn it, so a later hover still fades.
    await waitFor(() => expect(button()).not.toHaveAttribute('data-still'));

    rerender(<RailHarness />);
    expect(button()).toHaveAttribute('aria-expanded', 'true');
    expect(button()).toHaveAttribute('data-still', '');
    await waitFor(() => expect(button()).not.toHaveAttribute('data-still'));
  });

  it('snaps when the rail hides in rail mode without a press (a preview, the sidebar)', () => {
    const { rerender } = render(<Harness mode="rail" railShown railRendered />);
    rerender(<Harness mode="rail" />);
    expect(button()).toHaveAttribute('aria-expanded', 'false');
    expect(button()).toHaveAttribute('data-still', '');
  });

  it('keeps the fade for the person’s own press, both ways', async () => {
    const user = userEvent.setup();
    const onToggle = vi.fn();
    render(<RailHarness onToggle={onToggle} />);

    await user.click(button());
    expect(onToggle).toHaveBeenCalledTimes(1);
    expect(button()).toHaveAttribute('aria-expanded', 'false');
    expect(button()).not.toHaveAttribute('data-still');

    await user.click(button());
    expect(button()).toHaveAttribute('aria-expanded', 'true');
    expect(button()).not.toHaveAttribute('data-still');
  });

  it('keeps the fade for the popover, opened by a press and closed by Escape', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(button());
    await screen.findByRole('dialog');
    expect(button()).toHaveAttribute('aria-expanded', 'true');
    expect(button()).not.toHaveAttribute('data-still');

    await user.keyboard('{Escape}');
    await waitFor(() => expect(button()).toHaveAttribute('aria-expanded', 'false'));
    expect(button()).not.toHaveAttribute('data-still');
  });

  it('keeps the fade when the first turn brings the rail in', () => {
    const { rerender } = render(<Harness mode="rail" />);
    rerender(<Harness mode="rail" railShown railRendered railStill={false} />);
    expect(button()).toHaveAttribute('aria-expanded', 'true');
    expect(button()).not.toHaveAttribute('data-still');
  });

  it('does not let a press that changed nothing vouch for the next change', async () => {
    const user = userEvent.setup();
    // Rail mode before a turn: the press records the preference, nothing shows.
    const { rerender } = render(<Harness mode="rail" />);
    await user.click(button());
    expect(button()).toHaveAttribute('aria-expanded', 'false');
    rerender(<Harness mode="rail" railShown railRendered />);
    expect(button()).toHaveAttribute('data-still', '');
  });
});
