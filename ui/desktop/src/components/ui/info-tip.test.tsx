import { act, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { InfoTip, INFO_TIP_OPEN_DELAY_MS, useInfoTipId } from './info-tip';
import { Badge } from './badge';

const HELP = 'Checks spelling in the chat input.';

const surface = () => document.querySelector('[data-slot="info-tip-content"]');
const trigger = () => screen.getByRole('button', { name: 'About Spellcheck' });

function Page() {
  return (
    <div>
      <button type="button">Before</button>
      <span>Spellcheck</span>
      <InfoTip label="Spellcheck" help={HELP} />
    </div>
  );
}

describe('InfoTip', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('is named "About {label}" and described by a hidden node that exists while closed', () => {
    render(<Page />);
    expect(trigger()).toHaveAccessibleName('About Spellcheck');
    expect(trigger()).toHaveAccessibleDescription(HELP);
    const describedBy = trigger().getAttribute('aria-describedby');
    expect(describedBy).toBeTruthy();
    const node = document.getElementById(describedBy!);
    expect(node).toHaveTextContent(HELP);
    expect(node).toHaveClass('sr-only');
    expect(surface()).toBeNull();
    // A copy of the text stays findable, so tests that read the old paragraph keep passing.
    expect(screen.getByText(HELP)).toBeInTheDocument();
  });

  it('does not open before 200ms of hover, and opens after', () => {
    vi.useFakeTimers();
    render(<Page />);
    fireEvent.pointerEnter(trigger(), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(INFO_TIP_OPEN_DELAY_MS - 10);
    });
    expect(surface()).toBeNull();
    act(() => {
      vi.advanceTimersByTime(20);
    });
    expect(surface()).toHaveTextContent(HELP);
  });

  it('closes after the pointer leaves, with a short grace', () => {
    vi.useFakeTimers();
    render(<Page />);
    fireEvent.pointerEnter(trigger(), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(INFO_TIP_OPEN_DELAY_MS + 10);
    });
    expect(surface()).not.toBeNull();
    fireEvent.pointerLeave(trigger(), { pointerType: 'mouse' });
    expect(surface()).not.toBeNull();
    act(() => {
      vi.advanceTimersByTime(150);
    });
    expect(surface()?.getAttribute('data-state') ?? 'closed').toBe('closed');
  });

  it('opens at once when Tab moves focus onto it', async () => {
    const user = userEvent.setup();
    render(<Page />);
    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(trigger()).toHaveFocus();
    expect(surface()).toHaveTextContent(HELP);
  });

  it('stays shut when a program focuses it (a focus restore)', async () => {
    render(<Page />);
    act(() => trigger().focus());
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(surface()).toBeNull();
  });

  it('toggles on click and closes on Escape', async () => {
    const user = userEvent.setup();
    render(<Page />);
    await user.click(trigger());
    expect(surface()).toHaveTextContent(HELP);
    await user.click(trigger());
    expect(surface()?.getAttribute('data-state') ?? 'closed').toBe('closed');

    await user.click(trigger());
    expect(surface()).not.toBeNull();
    await user.keyboard('{Escape}');
    expect(surface()?.getAttribute('data-state') ?? 'closed').toBe('closed');
  });

  it('does not let a click reach a row that handles clicks', async () => {
    const user = userEvent.setup();
    const onRowClick = vi.fn();
    render(
      <div onClick={onRowClick}>
        <InfoTip label="Spellcheck" help={HELP} />
      </div>
    );
    await user.click(trigger());
    expect(onRowClick).not.toHaveBeenCalled();
  });

  it('lets the explained control point at the same description with useInfoTipId', () => {
    function Row() {
      const id = useInfoTipId();
      return (
        <div>
          <button type="button" aria-describedby={id}>
            Spellcheck switch
          </button>
          <InfoTip label="Spellcheck" help={HELP} id={id} />
        </div>
      );
    }
    render(<Row />);
    expect(screen.getByRole('button', { name: 'Spellcheck switch' })).toHaveAccessibleDescription(
      HELP
    );
  });

  it('makes a badge the focusable trigger with asChild, keeping its own name', () => {
    render(
      <InfoTip label="Legacy" help="Made before the format existed." asChild>
        <Badge>Legacy</Badge>
      </InfoTip>
    );
    const badge = screen.getByText('Legacy');
    expect(badge).toHaveAttribute('tabindex', '0');
    expect(badge).toHaveClass('br-info-tip-target');
    expect(badge).toHaveAccessibleDescription('Made before the format existed.');
  });

  it('keeps the base focus fill and inset edge off an asChild target (its outline is the mark)', () => {
    render(
      <InfoTip label="Legacy" help="Made before the format existed." asChild>
        <Badge>Legacy</Badge>
      </InfoTip>
    );
    // `tabindex="0"` would otherwise match the base D-15 rule's `[tabindex]` arm; this class is
    // that rule's documented opt-out. Measured in controlsGeometry.browser.test.tsx.
    expect(screen.getByText('Legacy')).toHaveClass('biorouter-focus-region');
  });

  it('accepts the help as children', () => {
    render(<InfoTip label="Spellcheck">{HELP}</InfoTip>);
    expect(trigger()).toHaveAccessibleDescription(HELP);
  });
});

/**
 * "Hover peeks, click pins" (docs/desktop-ui/shared-controls.md): a hover or a Tab PREVIEWS the
 * tip and leaves the trigger unmarked; a click, tap, Enter or Space KEEPS the same tip, marks the
 * trigger with `data-pinned`, and the kept tip outlives what closes a preview (pointer leave, a
 * scroll, a window switch's blur) until Escape, a press outside, a second click or focus that
 * moves elsewhere in the page.
 */
describe('InfoTip: hover peeks, click pins', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  const isOpen = () => (surface()?.getAttribute('data-state') ?? 'closed') === 'open';

  function hoverOpen() {
    fireEvent.pointerEnter(trigger(), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(INFO_TIP_OPEN_DELAY_MS + 10);
    });
  }

  it('marks only a kept tip: a hover preview leaves the trigger unmarked', () => {
    vi.useFakeTimers();
    render(<Page />);
    expect(trigger()).not.toHaveAttribute('data-pinned');
    hoverOpen();
    expect(isOpen()).toBe(true);
    // The preview is open, and the trigger still carries no mark.
    expect(trigger()).not.toHaveAttribute('data-pinned');
    expect(trigger()).toHaveAttribute('data-state', 'open');

    // A click on the previewed trigger keeps the very same tip.
    fireEvent.click(trigger());
    expect(isOpen()).toBe(true);
    expect(trigger()).toHaveAttribute('data-pinned', 'true');

    // A second click closes it, and the mark goes with it.
    fireEvent.click(trigger());
    expect(isOpen()).toBe(false);
    expect(trigger()).not.toHaveAttribute('data-pinned');
  });

  it('keeps a kept tip when the pointer leaves, and closes a preview', () => {
    vi.useFakeTimers();
    render(<Page />);
    hoverOpen();
    fireEvent.click(trigger());
    fireEvent.pointerLeave(trigger(), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(500);
    });
    expect(isOpen(), 'kept: pointer leave never closes it').toBe(true);

    fireEvent.click(trigger());
    hoverOpen();
    fireEvent.pointerLeave(trigger(), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(500);
    });
    expect(isOpen(), 'a preview closes after its grace').toBe(false);
  });

  it('keeps a kept tip through the blur a window switch causes, and closes a preview on blur', async () => {
    const user = userEvent.setup();
    render(<Page />);
    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(isOpen(), 'Tab previews it').toBe(true);
    expect(trigger()).not.toHaveAttribute('data-pinned');
    // Blur with nowhere for focus to go: what switching windows does to a focused trigger.
    act(() => {
      fireEvent.blur(trigger());
    });
    expect(isOpen(), 'a preview closes on blur').toBe(false);

    await user.click(trigger());
    expect(trigger()).toHaveAttribute('data-pinned', 'true');
    act(() => {
      fireEvent.blur(trigger());
    });
    expect(isOpen(), 'a kept tip outlives the blur').toBe(true);
    expect(trigger()).toHaveAttribute('data-pinned', 'true');
  });

  it('closes a kept tip when focus really moves to something else in the page', async () => {
    const user = userEvent.setup();
    render(
      <div>
        <InfoTip label="Spellcheck" help={HELP} />
        <button type="button">After</button>
      </div>
    );
    await user.click(trigger());
    expect(isOpen()).toBe(true);
    await user.tab();
    expect(screen.getByRole('button', { name: 'After' })).toHaveFocus();
    expect(isOpen()).toBe(false);
    expect(trigger()).not.toHaveAttribute('data-pinned');
  });

  it('keeps a kept tip through an ancestor scroll, and closes a preview on one', () => {
    vi.useFakeTimers();
    render(
      <div data-testid="scroller">
        <Page />
      </div>
    );
    const scroller = screen.getByTestId('scroller');
    hoverOpen();
    fireEvent.scroll(scroller);
    expect(isOpen(), 'a preview closes when its ancestor scrolls').toBe(false);

    fireEvent.click(trigger());
    expect(isOpen()).toBe(true);
    fireEvent.scroll(scroller);
    fireEvent.scroll(window);
    expect(isOpen(), 'a kept tip ignores the scroll').toBe(true);
  });

  it('closes a kept tip on Escape and on a press outside', async () => {
    const user = userEvent.setup();
    render(<Page />);
    await user.click(trigger());
    await user.keyboard('{Escape}');
    expect(isOpen()).toBe(false);
    expect(trigger()).not.toHaveAttribute('data-pinned');

    await user.click(trigger());
    expect(isOpen()).toBe(true);
    await user.click(screen.getByText('Spellcheck'));
    expect(isOpen()).toBe(false);
  });

  it('keeps a preview the person presses on', () => {
    vi.useFakeTimers();
    render(<Page />);
    hoverOpen();
    fireEvent.pointerDown(surface()!, { pointerType: 'mouse' });
    expect(trigger()).toHaveAttribute('data-pinned', 'true');
    fireEvent.pointerLeave(surface()!, { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(500);
    });
    expect(isOpen()).toBe(true);
  });

  it('never previews on touch; a tap keeps it and a second tap closes it', () => {
    vi.useFakeTimers();
    render(<Page />);
    fireEvent.pointerEnter(trigger(), { pointerType: 'touch' });
    act(() => {
      vi.advanceTimersByTime(INFO_TIP_OPEN_DELAY_MS + 10);
    });
    expect(surface()).toBeNull();
    fireEvent.click(trigger());
    expect(trigger()).toHaveAttribute('data-pinned', 'true');
    fireEvent.pointerLeave(trigger(), { pointerType: 'touch' });
    act(() => {
      vi.advanceTimersByTime(500);
    });
    expect(isOpen()).toBe(true);
    fireEvent.click(trigger());
    expect(isOpen()).toBe(false);
  });

  it('keeps and closes an asChild tip with Enter and Space, which no span does by itself', async () => {
    const user = userEvent.setup();
    const onRowKey = vi.fn();
    render(
      <div onKeyDown={onRowKey}>
        <button type="button">Before</button>
        <InfoTip label="Legacy" help="Made before the format existed." asChild>
          <Badge>Legacy</Badge>
        </InfoTip>
      </div>
    );
    const badge = screen.getByText('Legacy');
    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(badge).toHaveFocus();
    expect(isOpen(), 'Tab previews it').toBe(true);
    expect(badge).not.toHaveAttribute('data-pinned');
    onRowKey.mockClear();

    await user.keyboard('{Enter}');
    expect(badge).toHaveAttribute('data-pinned', 'true');
    await user.keyboard(' ');
    expect(isOpen()).toBe(false);
    expect(badge).not.toHaveAttribute('data-pinned');
    await user.keyboard(' ');
    expect(badge).toHaveAttribute('data-pinned', 'true');
    // The keys are the tip's: a row's own key handler never sees them.
    expect(onRowKey).not.toHaveBeenCalled();
  });

  // The glyph is a native <button> with no key handler of the tip's at all: its Enter and Space are
  // the button's own click, once per press.
  it('toggles a glyph once per Enter or Space, through its own button’s click', async () => {
    const user = userEvent.setup();
    render(<Page />);
    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    await user.keyboard('{Enter}');
    expect(trigger()).toHaveAttribute('data-pinned', 'true');
    await user.keyboard(' ');
    expect(isOpen()).toBe(false);
  });

  // An `asChild` target that is ITSELF a native control does get the tip's key handler, which
  // must stand aside (`NATIVELY_ACTIVATED`): the control's own Enter and Space click it, the click
  // toggles the tip, and the control's own click still runs. Taking the keys there would cancel
  // the control's activation, so its own job never ran from the keyboard, and would hide them
  // from the row around it.
  it('leaves an asChild native control its own keys: one toggle per press, and its click runs', async () => {
    const user = userEvent.setup();
    const onOwnClick = vi.fn();
    const onRowKey = vi.fn();
    render(
      <div onKeyDown={onRowKey}>
        <button type="button">Before</button>
        <InfoTip label="Plan" help="What the plan covers." asChild>
          <button type="button" onClick={onOwnClick}>
            Plan
          </button>
        </InfoTip>
      </div>
    );
    const plan = screen.getByRole('button', { name: 'Plan' });
    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(plan).toHaveFocus();
    expect(isOpen(), 'Tab previews it').toBe(true);
    onRowKey.mockClear();

    await user.keyboard('{Enter}');
    expect(plan).toHaveAttribute('data-pinned', 'true');
    expect(onOwnClick, 'Enter clicked the control, once').toHaveBeenCalledTimes(1);
    await user.keyboard('{Enter}');
    expect(isOpen(), 'a second Enter closes it').toBe(false);
    expect(onOwnClick).toHaveBeenCalledTimes(2);

    await user.keyboard(' ');
    expect(plan).toHaveAttribute('data-pinned', 'true');
    expect(onOwnClick, 'Space clicked the control, once').toHaveBeenCalledTimes(3);
    await user.keyboard(' ');
    expect(isOpen(), 'a second Space closes it').toBe(false);
    expect(onOwnClick).toHaveBeenCalledTimes(4);
    // The keys were the control's, not taken: the row around it heard every one.
    expect(onRowKey).toHaveBeenCalledTimes(4);
  });
});
