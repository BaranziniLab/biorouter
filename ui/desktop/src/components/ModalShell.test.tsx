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
import { InfoTip } from './ui/info-tip';

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

    // The scrim shares the surface's layer and comes before it, so portal order stacks a dialog
    // over its own scrim and a dialog opened from it over both (V4-08,
    // `ui/dialogStacking.browser.test.tsx`).
    const overlay = document.querySelector('[data-slot="dialog-overlay"]')!;
    expect(overlay).toHaveClass('z-[var(--z-modal)]');
    expect(
      overlay.compareDocumentPosition(content) & Node.DOCUMENT_POSITION_FOLLOWING
    ).toBeTruthy();
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

describe('ModalShell — a test id on the surface (V3-R7-1)', () => {
  it('puts testId on the dialog node, and no attribute without one', () => {
    const { rerender } = render(
      <ModalShell open onOpenChange={() => {}} title="Disclosure" testId="the-disclosure" />
    );
    expect(screen.getByRole('dialog')).toHaveAttribute('data-testid', 'the-disclosure');
    rerender(<ModalShell open onOpenChange={() => {}} title="Disclosure" />);
    expect(screen.getByRole('dialog')).not.toHaveAttribute('data-testid');
  });
});

describe('ModalShell — xl', () => {
  // Spec 2.6: an 800px size for editors. Named on the surface so main.css can author the width,
  // since a newly written arbitrary utility can fail to generate under BIOROUTER_NO_HMR.
  it('names its size on the surface, and xl is 800px', () => {
    open({ size: 'xl' });
    expect(surface()).toHaveAttribute('data-size', 'xl');
    expect(MODAL_SIZE.xl).toBe('sm:max-w-[800px]');
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

  // QA Q2-28: a dialog whose body is only a message was announced by its title alone.
  it('names a message body as the description when asked, without a warning', () => {
    render(
      <ModalShell open onOpenChange={vi.fn()} title="Add people to #general" describedBy="note-1">
        <p id="note-1">Everyone in Analysis Lab is already here.</p>
      </ModalShell>
    );
    expect(surface()).toHaveAttribute('aria-describedby', 'note-1');
    expect(screen.getByRole('dialog')).toHaveAccessibleDescription(
      'Everyone in Analysis Lab is already here.'
    );
    expect(warned()).toBe(false);
  });

  it('keeps the subtitle as the description when both are given', () => {
    open({ subtitle: 'in Analysis Lab', describedBy: 'note-1' });
    expect(screen.getByRole('dialog')).toHaveAccessibleDescription('in Analysis Lab');
  });
});

describe('ModalShell — an InfoTip beside the title (V4-R4-2)', () => {
  it('sits on the title line, outside the heading, so the dialog keeps the title as its name', () => {
    open({
      title: 'Report a problem',
      titleInfo: <InfoTip label="Report a problem" help="Say report a bug in the chat." />,
    });
    const dialog = screen.getByRole('dialog', { name: 'Report a problem' });
    const heading = screen.getByRole('heading', { name: 'Report a problem' });
    const tip = screen.getByRole('button', { name: 'About Report a problem' });
    expect(heading).not.toContainElement(tip);
    // One row: the heading and the tip share a parent inside the header band, which keeps its rule.
    expect(tip.parentElement).toBe(heading.parentElement);
    expect(heading.parentElement!.parentElement).toHaveClass('border-b');
    expect(dialog).toContainElement(tip);
  });

  it('leaves the header as it was without one: the heading is the band’s first child', () => {
    open();
    const heading = screen.getByRole('heading', { name: 'Do the thing' });
    expect(heading.parentElement).toHaveClass('border-b');
  });
});

describe('ModalShell — defaults an area gives its dialogs', () => {
  const header = () => surface().firstElementChild as HTMLElement;

  // QA Q2-25: a dialog portals outside the area that mounts it, so the area's stylesheet reaches
  // it only through a class on the dialog itself.
  it('adds the area’s class to the content, beside the shell’s own', () => {
    render(
      <ModalShellDefaultsContext.Provider value={{ className: 'crew-dialog' }}>
        <ModalShell open onOpenChange={vi.fn()} title="Inside Crew" className="extra" />
      </ModalShellDefaultsContext.Provider>
    );
    expect(surface()).toHaveClass('crew-dialog', 'extra');
  });

  // QA Q2-26: one header rule for every dialog of an area, not only the scrolling ones. A context
  // value is still honoured (an area can opt its dialogs out), and a prop still beats it.
  it('draws the header hairline from a default, a prop winning, and gives the body its gutter', () => {
    render(
      <ModalShellDefaultsContext.Provider value={{ headerRule: true }}>
        <ModalShell open onOpenChange={vi.fn()} title="Inside Crew">
          <p>body</p>
        </ModalShell>
      </ModalShellDefaultsContext.Provider>
    );
    expect(header()).toHaveClass('border-b', 'border-border-subtle');
    expect(screen.getByText('body').parentElement).toHaveClass('pt-3');
    cleanup();

    render(
      <ModalShellDefaultsContext.Provider value={{ headerRule: true }}>
        <ModalShell open onOpenChange={vi.fn()} title="Its own choice" headerRule={false} />
      </ModalShellDefaultsContext.Provider>
    );
    expect(header()).not.toHaveClass('border-b');
    cleanup();

    render(
      <ModalShellDefaultsContext.Provider value={{ headerRule: false }}>
        <ModalShell open onOpenChange={vi.fn()} title="An area that opts out">
          <p>body</p>
        </ModalShell>
      </ModalShellDefaultsContext.Provider>
    );
    expect(header()).not.toHaveClass('border-b');
    cleanup();

    render(
      <ModalShellDefaultsContext.Provider value={{ headerRule: false }}>
        <ModalShell open onOpenChange={vi.fn()} title="Its own choice" headerRule>
          <p>body</p>
        </ModalShell>
      </ModalShellDefaultsContext.Provider>
    );
    expect(header()).toHaveClass('border-b');
  });
});

/**
 * Coordinator ruling 1 (Codex simplicity redesign): Crew is the reference design, so the header
 * hairline is the shell's own default, not something an area has to provide. A dialog outside Crew,
 * with no provider above it, gets the same chrome.
 */
describe('ModalShell — the header rule is on by default', () => {
  const header = () => surface().firstElementChild as HTMLElement;

  it('draws the hairline with no provider and no prop, and gives the body its gutter', () => {
    open();
    expect(header()).toHaveClass('border-b', 'border-border-subtle');
    expect(screen.getByText('body').parentElement).toHaveClass('pt-3');
  });

  it('draws it for a dialog that is only a title and a footer', () => {
    render(
      <ModalShell
        open
        onOpenChange={vi.fn()}
        title="Delete this?"
        footer={<button>Delete</button>}
      />
    );
    expect(header()).toHaveClass('border-b');
  });

  it('keeps the rule for a scrolling body whatever headerRule says', () => {
    open({ scrollBody: true, headerRule: false });
    expect(header()).toHaveClass('border-b');
  });

  it('headerRule={false} is the explicit opt-out', () => {
    open({ headerRule: false });
    expect(header()).not.toHaveClass('border-b');
    // No rule, so the body takes no extra top gutter.
    expect(screen.getByText('body').parentElement).not.toHaveClass('pt-3');
  });
});

/**
 * The anchor axis (QA T-30). The primitive centres a dialog with `top: 50%` and a -50% Y translate,
 * so a dialog whose height changes while it is open re-centres and jumps under the pointer. `top`
 * pins the top edge instead, and since coordinator ruling 1 it is the default everywhere: Crew's
 * placement is the reference design.
 *
 * jsdom drops `max()` from an inline `top`, so the geometry is asserted on the one exported style
 * (what a real browser receives) and the rendered surface is asserted by its marker and translate.
 */
describe('ModalShell — the anchor axis', () => {
  it('pins the top edge by default, with no provider and no prop', () => {
    open();
    expect(surface()).toHaveAttribute('data-anchor', 'top');
    expect((surface() as HTMLElement).style.getPropertyValue('translate')).toBe('-50% 0');
    expect(MODAL_ANCHOR_TOP_STYLE).toEqual({
      top: 'max(10vh, 48px)',
      translate: '-50% 0',
      maxHeight: 'min(85vh, calc(100vh - max(10vh, 48px) - 16px))',
    });
  });

  it('still centres when a dialog asks for it, with no inline geometry', () => {
    open({ anchor: 'center' });
    expect(surface()).not.toHaveAttribute('data-anchor');
    expect(surface().getAttribute('style') ?? '').not.toContain('translate');
    expect(surface()).toHaveClass('top-[50%]', 'translate-y-[-50%]');
  });

  it('pins the top edge and drops the Y translate for `top`', () => {
    open({ anchor: 'top' });
    expect(surface()).toHaveAttribute('data-anchor', 'top');
    expect((surface() as HTMLElement).style.getPropertyValue('translate')).toBe('-50% 0');
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
    cleanup();

    // An area can still ask for centre, and the shell's `top` default does not override it.
    render(
      <ModalShellDefaultsContext.Provider value={{ anchor: 'center' }}>
        <ModalShell open onOpenChange={vi.fn()} title="An area that centres" />
      </ModalShellDefaultsContext.Provider>
    );
    expect(surface()).not.toHaveAttribute('data-anchor');
  });
});

describe('ModalShell — where focus lands on open (WS-VIEWS-A-4, V3-20)', () => {
  it('hands Radix’s open-focus hook to the caller, so a form of Selects can focus the dialog', () => {
    const onOpenAutoFocus = vi.fn((event: Event) => {
      event.preventDefault();
      (event.currentTarget as HTMLElement).focus();
    });
    render(
      <ModalShell open onOpenChange={vi.fn()} title="Schedule" onOpenAutoFocus={onOpenAutoFocus}>
        <button type="button">Every day</button>
      </ModalShell>
    );
    expect(onOpenAutoFocus).toHaveBeenCalledTimes(1);
    expect(surface()).toHaveFocus();
    expect(screen.getByRole('button', { name: 'Every day' })).not.toHaveFocus();
  });

  it('leaves Radix’s first-tabbable focus alone without one', () => {
    render(
      <ModalShell open onOpenChange={vi.fn()} title="Schedule">
        <button type="button">Every day</button>
      </ModalShell>
    );
    expect(screen.getByRole('button', { name: 'Every day' })).toHaveFocus();
  });
});
