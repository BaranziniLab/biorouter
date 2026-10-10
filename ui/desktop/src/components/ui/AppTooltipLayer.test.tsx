import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { AppTooltipLayer } from './AppTooltipLayer';

function NativeTitleTarget({ title }: { title?: string }) {
  return (
    <button type="button" data-testid="native-title-target" title={title}>
      <span aria-hidden="true">?</span>
    </button>
  );
}

describe('AppTooltipLayer', () => {
  it('upgrades native titles to the Biorouter tooltip surface', async () => {
    render(
      <>
        <AppTooltipLayer />
        <NativeTitleTarget title="Native action" />
      </>
    );

    const target = screen.getByTestId('native-title-target');
    await waitFor(() => expect(target).toHaveAttribute('title', ''));
    expect(target).toHaveAttribute('data-biorouter-tooltip', 'Native action');
    expect(target).toHaveAccessibleName('Native action');

    fireEvent.pointerOver(target);
    const tooltip = await screen.findByRole('tooltip');
    expect(tooltip).toHaveTextContent('Native action');
    // `rounded-container` (12px), not `rounded-inner` (4px): a tooltip is a
    // floating surface, and both the radius ladder in `main.css` and the
    // cohesion design put every floating surface — popover, dropdown, select,
    // mention picker, toast, tooltip — on one 12px recipe. The 4px this used to
    // assert is the role reserved for things nested INSIDE a control.
    expect(tooltip).toHaveClass(
      'bg-background-inverse',
      'text-text-inverse',
      'rounded-container',
      'font-sans'
    );
    expect(tooltip).not.toHaveClass('text-balance');
    expect(tooltip.querySelector('[aria-hidden="true"]')).toBeNull();
  });

  it('keeps dynamic tooltip text and generated accessible names synchronized', async () => {
    const { rerender } = render(
      <>
        <AppTooltipLayer />
        <NativeTitleTarget title="First action" />
      </>
    );

    const target = screen.getByTestId('native-title-target');
    await waitFor(() => expect(target).toHaveAttribute('data-biorouter-tooltip', 'First action'));

    rerender(
      <>
        <AppTooltipLayer />
        <NativeTitleTarget title="Updated action" />
      </>
    );
    await waitFor(() => expect(target).toHaveAttribute('data-biorouter-tooltip', 'Updated action'));
    expect(target).toHaveAccessibleName('Updated action');

    rerender(
      <>
        <AppTooltipLayer />
        <NativeTitleTarget />
      </>
    );
    await waitFor(() => expect(target).not.toHaveAttribute('data-biorouter-tooltip'));
    expect(target).not.toHaveAttribute('aria-label');
  });

  it('preserves intentional line breaks in compact native-title tooltips', async () => {
    render(
      <>
        <AppTooltipLayer />
        <NativeTitleTarget title={'Ships with Biorouter.\nRecreated automatically if deleted.'} />
      </>
    );

    const target = screen.getByTestId('native-title-target');
    await waitFor(() => expect(target).toHaveAttribute('title', ''));

    fireEvent.pointerOver(target);
    const tooltip = await screen.findByRole('tooltip');
    expect(tooltip).toHaveTextContent('Ships with Biorouter. Recreated automatically if deleted.', {
      normalizeWhitespace: true,
    });
    expect(tooltip).toHaveClass('whitespace-pre-line', 'text-left', 'text-supporting');
  });

  it('uses intrinsic width for short action labels', async () => {
    render(
      <>
        <AppTooltipLayer />
        <NativeTitleTarget title="Delete local model" />
      </>
    );

    const target = screen.getByTestId('native-title-target');
    await waitFor(() => expect(target).toHaveAttribute('title', ''));

    fireEvent.pointerOver(target);
    const tooltip = await screen.findByRole('tooltip');
    expect(tooltip).toHaveTextContent('Delete local model');
    expect(tooltip).toHaveClass('w-max', 'max-w-[min(20rem,calc(100vw-16px))]', 'break-words');
  });

  /**
   * The stranded tooltip. Hover a row control, then change route without moving
   * the pointer: the target unmounts, so no `pointerout` is ever delivered and
   * the tooltip is left on screen describing an element that no longer exists.
   *
   * The check for this was `useEffect(…, [tooltip])` — it ran when the tooltip
   * STATE changed, which is never the case here.
   */
  it('dismisses a tooltip whose target leaves the page', async () => {
    const { rerender } = render(
      <>
        <AppTooltipLayer />
        <NativeTitleTarget title="Native action" />
      </>
    );

    const target = screen.getByTestId('native-title-target');
    await waitFor(() => expect(target).toHaveAttribute('data-biorouter-tooltip', 'Native action'));
    fireEvent.pointerOver(target);
    expect(await screen.findByRole('tooltip')).toHaveTextContent('Native action');

    // The route change. The pointer never moves, so the only signal is the
    // removal itself.
    rerender(<AppTooltipLayer />);

    await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
  });

  it('does not open after the pointer leaves during the delay', async () => {
    render(
      <>
        <AppTooltipLayer />
        <NativeTitleTarget title="Delayed action" />
      </>
    );

    const target = screen.getByTestId('native-title-target');
    await waitFor(() => expect(target).toHaveAttribute('title', ''));

    fireEvent.pointerOver(target);
    fireEvent.pointerOut(target);
    await new Promise((resolve) => window.setTimeout(resolve, 550));

    expect(screen.queryByRole('tooltip')).not.toBeInTheDocument();
  });

  /**
   * Spec 2.5 / primitives defect 7: the layer listened to every `focusin`, so a dialog or menu
   * handing focus back to its opener (or a click that focuses a button) popped the opener's title
   * tooltip, the Q2-56 bug the Radix Tooltip already fixed. It now uses the same Tab gate.
   */
  it('stays shut when a program focuses a titled control', async () => {
    render(
      <>
        <AppTooltipLayer />
        <NativeTitleTarget title="Native action" />
      </>
    );
    const target = screen.getByTestId('native-title-target');
    await waitFor(() => expect(target).toHaveAttribute('data-biorouter-tooltip', 'Native action'));

    act(() => target.focus());
    expect(target).toHaveFocus();
    await act(() => new Promise((resolve) => setTimeout(resolve, 600)));
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('opens when Tab moves focus onto a titled control', async () => {
    const user = userEvent.setup();
    render(
      <>
        <AppTooltipLayer />
        <button type="button">Before</button>
        <NativeTitleTarget title="Native action" />
      </>
    );
    const target = screen.getByTestId('native-title-target');
    await waitFor(() => expect(target).toHaveAttribute('data-biorouter-tooltip', 'Native action'));
    act(() => screen.getByRole('button', { name: 'Before' }).focus());

    await user.tab();

    expect(target).toHaveFocus();
    expect(await screen.findByRole('tooltip', {}, { timeout: 2000 })).toHaveTextContent(
      'Native action'
    );
  });

  it('fades on the motion tokens, not a literal duration', async () => {
    render(
      <>
        <AppTooltipLayer />
        <NativeTitleTarget title="Native action" />
      </>
    );
    const target = screen.getByTestId('native-title-target');
    await waitFor(() => expect(target).toHaveAttribute('data-biorouter-tooltip', 'Native action'));
    fireEvent.pointerOver(target);
    const tooltip = await screen.findByRole('tooltip');
    expect(tooltip).toHaveClass('br-tooltip-motion');
    expect(tooltip.className).not.toMatch(/duration-\[/);
  });
});
