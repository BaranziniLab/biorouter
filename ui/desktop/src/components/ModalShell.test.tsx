import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest';
import {
  MODAL_ANCHOR_TOP_STYLE,
  MODAL_SIZE,
  ModalShell,
  ModalShellDefaultsContext,
  type ModalPurpose,
} from './ModalShell';

afterEach(cleanup);

function open(props: Partial<React.ComponentProps<typeof ModalShell>> = {}) {
  const onOpenChange = vi.fn();
  render(
    <ModalShell open onOpenChange={onOpenChange} title="Do the thing" {...props}>
      <p>body</p>
    </ModalShell>
  );
  return { onOpenChange };
}

const surface = () => document.querySelector('[data-slot="dialog-content"]')!;

describe('ModalShell — the structural guarantee', () => {
  // A modal whose content loses to its own scrim covers the whole app and reads
  // as a freeze; the app has shipped that bug. `.biorouter-modal-surface` is the
  // unlayered `z-index: var(--z-modal)` floor that makes it unreachable, and it
  // arrives only because the shell renders through the Radix `DialogContent`.
  // Nothing here may paint its own overlay or set its own z-index.
  it('renders on the primitive that carries the z-index floor, and sets no z-index of its own', () => {
    open();
    const content = surface();
    expect(content).toHaveClass('biorouter-modal-surface');
    // The one z-* class allowed is the primitive's own --z-modal.
    const zClasses = Array.from(content.classList).filter((c) => /^z-/.test(c));
    expect(zClasses).toEqual(['z-[var(--z-modal)]']);

    const overlay = document.querySelector('[data-slot="dialog-overlay"]')!;
    expect(overlay).toHaveClass('z-[var(--z-overlay)]');
  });
});

describe('ModalShell — the size scale', () => {
  it.each(Object.entries(MODAL_SIZE))('%s maps to exactly one width', (size, className) => {
    open({ size: size as keyof typeof MODAL_SIZE });
    const widths = Array.from(surface().classList).filter((c) => c.includes('max-w'));
    // The primitive's default `sm:max-w-lg` must have been merged away, leaving
    // the scale's width plus the unprefixed small-screen clamp.
    expect(widths).toContain(className);
    expect(widths).not.toContain('sm:max-w-lg');
  });
});

describe('ModalShell — the purpose axis', () => {
  it('info dismisses on Escape', async () => {
    const user = userEvent.setup();
    const { onOpenChange } = open({ purpose: 'info' });
    await user.keyboard('{Escape}');
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  // The bug the axis exists to prevent: a half-filled form thrown away by a
  // misclick on the scrim.
  it('form ignores a backdrop click but keeps its Escape route', async () => {
    const user = userEvent.setup();
    const { onOpenChange } = open({ purpose: 'form' });

    await user.click(document.querySelector('[data-slot="dialog-overlay"]')!);
    expect(onOpenChange).not.toHaveBeenCalled();

    await user.keyboard('{Escape}');
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('required offers no close affordance and ignores Escape', async () => {
    const user = userEvent.setup();
    const { onOpenChange } = open({ purpose: 'required' });

    expect(screen.queryByRole('button', { name: 'Close' })).not.toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(onOpenChange).not.toHaveBeenCalled();
  });

  it.each<[ModalPurpose, boolean]>([
    ['info', true],
    ['form', true],
    ['required', false],
  ])('%s renders the single × exactly %s times', (purpose, shown) => {
    open({ purpose });
    // ONE close affordance: the primitive's ×, never a second hand-rolled one.
    expect(screen.queryAllByRole('button', { name: 'Close' })).toHaveLength(shown ? 1 : 0);
  });
});

/**
 * The description axis, pinned at both ends.
 *
 * ⚠ **The subtitle IS the description — one string, two audiences.** A modal
 * whose screen-reader description says something the visible line does not is a
 * worse defect than the console warning it silences, so the shell links the
 * paragraph it already renders rather than authoring a second one. `KBManagerDialog`
 * is the case that matters: its "Choose which knowledge bases this chat uses…"
 * line reaches the accessibility tree only through this wiring.
 *
 * These tests install a local warning spy because their assertions need to
 * distinguish a settled description from a warning that was merely hidden.
 */
describe('ModalShell — the description contract', () => {
  let warn: MockInstance;

  beforeEach(() => {
    warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
  });

  afterEach(() => {
    warn.mockRestore();
  });

  const warned = () =>
    warn.mock.calls.some((call) => String(call[0]).includes('Missing `Description`'));

  it('links the subtitle, so the description and the visible line are one string', () => {
    open({ subtitle: 'Choose which knowledge bases this chat uses.' });

    const id = surface().getAttribute('aria-describedby');
    expect(id).toBeTruthy();
    expect(document.getElementById(id!)).toHaveTextContent(
      'Choose which knowledge bases this chat uses.'
    );
    expect(warned()).toBe(false);
  });

  it('drops the attribute without a subtitle, rather than dangling it at nothing', () => {
    open();

    // Absent, not empty: an id that resolves to no element is the actual defect,
    // and it is what Radix warns about.
    expect(surface().hasAttribute('aria-describedby')).toBe(false);
    expect(warned()).toBe(false);
  });
});

/**
 * The anchor axis (QA T-30). The primitive centres a dialog with `top: 50%` and a -50% Y translate,
 * so a dialog whose height changes while it is open re-centres and jumps under the pointer. `top`
 * pins the top edge instead. The default stays `center`, so no caller outside Crew moves.
 *
 * jsdom drops `max()` from an inline `top`, so the geometry is asserted on the one exported style
 * (what a real browser receives) and the rendered surface is asserted by its marker and translate.
 */
describe('ModalShell — the anchor axis', () => {
  it('stays centred by default, with no inline geometry', () => {
    open();
    expect(surface()).not.toHaveAttribute('data-anchor');
    expect(surface().getAttribute('style') ?? '').not.toContain('translate');
    expect(surface()).toHaveClass('top-[50%]', 'translate-y-[-50%]');
  });

  it('pins the top edge and drops the Y translate for `top`', () => {
    open({ anchor: 'top' });
    expect(surface()).toHaveAttribute('data-anchor', 'top');
    expect((surface() as HTMLElement).style.getPropertyValue('translate')).toBe('-50% 0');
    expect(MODAL_ANCHOR_TOP_STYLE).toEqual({
      top: 'max(10vh, 48px)',
      translate: '-50% 0',
      maxHeight: 'min(85vh, calc(100vh - max(10vh, 48px) - 16px))',
    });
  });

  it('takes the anchor and close handler from a surrounding default, a prop winning', async () => {
    const user = userEvent.setup();
    const onCloseAutoFocus = vi.fn((event: Event) => event.preventDefault());
    const onOpenChange = vi.fn();
    const { rerender } = render(
      <ModalShellDefaultsContext.Provider value={{ anchor: 'top', onCloseAutoFocus }}>
        <ModalShell open onOpenChange={onOpenChange} title="Inside Crew">
          <p>body</p>
        </ModalShell>
      </ModalShellDefaultsContext.Provider>
    );
    expect(surface()).toHaveAttribute('data-anchor', 'top');
    await user.keyboard('{Escape}');
    rerender(
      <ModalShellDefaultsContext.Provider value={{ anchor: 'top', onCloseAutoFocus }}>
        <ModalShell open={false} onOpenChange={onOpenChange} title="Inside Crew">
          <p>body</p>
        </ModalShell>
      </ModalShellDefaultsContext.Provider>
    );
    await vi.waitFor(() => expect(onCloseAutoFocus).toHaveBeenCalled());
    cleanup();

    render(
      <ModalShellDefaultsContext.Provider value={{ anchor: 'top' }}>
        <ModalShell open onOpenChange={vi.fn()} anchor="center" title="Its own choice" />
      </ModalShellDefaultsContext.Provider>
    );
    expect(surface()).not.toHaveAttribute('data-anchor');
  });
});
