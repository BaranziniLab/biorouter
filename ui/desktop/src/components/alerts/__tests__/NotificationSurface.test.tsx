import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import {
  NotificationSurface,
  NotificationContent,
  TOAST_SURFACE_CLASS_NAME,
  type NotificationStatus,
} from '../NotificationSurface';

const STATUSES: NotificationStatus[] = ['success', 'error', 'warning', 'info', 'loading'];

describe('NotificationContent (shared layout primitive)', () => {
  it('reserves the close-button gutter on the toast card that React-Toastify always renders', () => {
    const classes = TOAST_SURFACE_CLASS_NAME.split(/\s+/);

    expect(classes).toEqual(expect.arrayContaining(['pl-3', 'pr-12', 'min-w-0']));
    expect(classes).not.toContain('px-3');
  });

  it('draws the toast as the one floating-surface recipe, never an in-page fill', () => {
    // A toast floats: hairline and shadow from `.biorouter-popover-surface` (lifted
    // in dark), on the page ground. R-27 keeps lines on floating surfaces.
    const classes = TOAST_SURFACE_CLASS_NAME.split(/\s+/);
    expect(classes).toEqual(
      expect.arrayContaining(['biorouter-popover-surface', 'bg-background-default'])
    );
    expect(classes).not.toContain('bg-background-fill');
    expect(classes).not.toContain('border-border-subtle');
  });

  it('renders the title and message together', () => {
    render(<NotificationContent status="success" title="Model changed" message="Now using Opus" />);
    expect(screen.getByText('Model changed')).toBeInTheDocument();
    expect(screen.getByText('Now using Opus')).toBeInTheDocument();
  });

  it('renders the message even when NO title is given (regression: message must never be dropped)', () => {
    render(<NotificationContent status="success" message="Workflow saved and ready to use." />);
    expect(screen.getByText('Workflow saved and ready to use.')).toBeInTheDocument();
  });

  it('renders a title with no message', () => {
    render(<NotificationContent status="success" title="All extensions loaded" />);
    expect(screen.getByText('All extensions loaded')).toBeInTheDocument();
  });

  it('renders a lone message at full strength, but mutes it beneath a title', () => {
    const { rerender } = render(<NotificationContent status="info" message="Standalone note" />);
    expect(screen.getByText('Standalone note').className).toContain('text-text-default');
    rerender(<NotificationContent status="info" title="Heads up" message="Standalone note" />);
    expect(screen.getByText('Standalone note').className).toContain('text-text-muted');
  });

  it('renders without throwing when neither title nor message is given, and still shows the status chip', () => {
    const { container } = render(<NotificationContent status="loading" />);
    expect(container.querySelector('[data-status="loading"]')).toBeInTheDocument();
  });

  it.each(STATUSES)('renders a status chip carrying its status and an icon for "%s"', (status) => {
    const { container } = render(<NotificationContent status={status} title="x" />);
    const chip = container.querySelector(`[data-status="${status}"]`);
    expect(chip).toBeInTheDocument();
    expect(chip!.querySelector('svg')).toBeInTheDocument();
  });

  it.each(STATUSES)(
    'centers the first text line against the 28px status chip for "%s"',
    (status) => {
      const { container } = render(<NotificationContent status={status} title="Aligned title" />);
      expect(container.querySelector('[data-notification-text]')).toHaveClass('pt-[5px]');
    }
  );

  it('top-aligns the status chip so it anchors to the first line at any height (never floats)', () => {
    const { container } = render(<NotificationContent status="error" title="t" message="m" />);
    const chip = container.querySelector('[data-status="error"]')!;
    expect(chip.className).toContain('self-start');
  });

  it('keeps long session names in a shrinkable, wrapping text column', () => {
    const { container } = render(
      <NotificationContent
        status="success"
        title="Session deleted"
        message={`"${'unbroken-session-name-'.repeat(12)}" was removed from chat history.`}
      />
    );

    const textColumn = container.querySelector('.min-w-0');
    expect(textColumn).toHaveClass('flex-1', 'min-w-0');
    expect(screen.getByText('Session deleted')).toHaveClass('[overflow-wrap:anywhere]');
    expect(screen.getByText(/was removed from chat history/)).toHaveClass(
      '[overflow-wrap:anywhere]'
    );
  });

  // (Σ) One geometry, two densities. A toast is sometimes title+body and sometimes
  // title-only; both must be the SAME object, sharing a top edge and a first-line
  // centre, with the two-line form growing downward. Nothing re-centres or jumps.
  describe('one geometry, two densities', () => {
    it('renders NO empty body node for a title-only notification', () => {
      const { container } = render(<NotificationContent status="info" title="Extension removed" />);
      const textColumn = container.querySelector('[data-notification-text]')!;

      // Exactly the title — no stray empty <div> holding the absent message open,
      // which would pad the "tidy 48px bar" out to a two-line height.
      expect(textColumn.children).toHaveLength(1);
      expect(textColumn.textContent).toBe('Extension removed');
    });

    it('adds the body as a sibling BELOW the title, leaving the first line untouched', () => {
      const { container } = render(
        <NotificationContent status="info" title="primekgagent" message="Extension removed" />
      );
      const textColumn = container.querySelector('[data-notification-text]')!;

      expect(textColumn.children).toHaveLength(2);
      expect(textColumn.children[0].textContent).toBe('primekgagent');
      // The body grows downward (mt-0.5); it never re-centres what is above it.
      expect(textColumn.children[1].className).toContain('mt-0.5');
    });

    it('keeps the chip and the text column top-anchored at BOTH densities, so the two share a top edge', () => {
      for (const props of [
        { status: 'info' as const, title: 'Extension removed' },
        { status: 'info' as const, title: 'primekgagent', message: 'Extension removed' },
      ]) {
        const { container, unmount } = render(<NotificationContent {...props} />);

        // `self-start` on the chip is what anchors it to the first line at ANY
        // height. Were it centre-aligned, the one-line and two-line toasts would
        // disagree about where the chip sits — the exact jump (Σ) forbids.
        // (`items-center` on the chip centres the glyph INSIDE its 28px box — that
        // is separate from, and does not affect, where the box itself sits.)
        const chip = container.querySelector('[data-status="info"]')!;
        expect(chip.className).toContain('self-start');
        expect(chip.className).not.toContain('self-center');
        // The toast wrapper top-aligns its row for the same reason.
        expect(TOAST_SURFACE_CLASS_NAME.split(/\s+/)).toContain('items-start');
        // 28px chip vs 13/18 text: pt-[5px] is what puts the first line's centre on
        // the chip's centre (5 + 18/2 === 28/2).
        expect(container.querySelector('[data-notification-text]')).toHaveClass('pt-[5px]');

        unmount();
      }
    });

    it('centres the dismiss control on the first line at both densities (never a tall banner’s midpoint)', () => {
      for (const props of [
        { status: 'info' as const, title: 'Extension removed' },
        { status: 'info' as const, title: 'primekgagent', message: 'Extension removed' },
      ]) {
        const { container, unmount } = render(
          <NotificationSurface {...props} onClose={() => {}} />
        );
        const dismiss = screen.getByRole('button', { name: /dismiss/i });

        // §4.2 compact close: 20px ghost, the 4px inner radius (`rounded-inner`;
        // `rounded-sm` is its deprecated alias), right-2.5, 14px icon.
        expect(dismiss.className).toContain('h-5');
        expect(dismiss.className).toContain('w-5');
        expect(dismiss.className).toContain('rounded-inner');
        expect(dismiss.className).toContain('right-2.5');
        expect(dismiss.querySelector('svg')!.getAttribute('class')).toContain('h-3.5');

        // The centring itself: container p-3 (12) + half the 28px chip = 26px, and a
        // 20px button centres there at top-4 (16 + 10 === 26). `top-4` is arithmetic,
        // not taste — if the surface padding ever changes, this must change with it.
        expect(
          container.querySelector('[data-testid="notification-surface"]')!.className
        ).toContain('p-3');
        expect(dismiss.className).toContain('top-4');
        // Not centred against the whole (possibly two-line) card.
        expect(dismiss.className).not.toContain('top-1/2');
        expect(dismiss.className).not.toContain('inset-y-0');

        unmount();
      }
    });

    it('keeps 12px radius and a neutral surface — the tinted chip carries status, not a left bar', () => {
      const { container } = render(
        <NotificationSurface status="error" title="Extension removed" />
      );
      const surface = container.querySelector('[data-testid="notification-surface"]')!;

      // Deliberate deviation from §4.3's --radius-lg: 12px matches every other
      // floating surface (popover / dropdown / select menu).
      expect(surface.className).toContain('rounded-container');
      expect(TOAST_SURFACE_CLASS_NAME.split(/\s+/)).toContain('rounded-container');

      // §4.3's 3px left status bar is deliberately NOT adopted — the chip is the
      // status signal. Guard against anyone reintroducing the stripe.
      expect(surface.className).not.toMatch(/border-l-|before:/);
      expect(container.querySelector('[data-status="error"]')).toBeInTheDocument();
    });
  });

  it('renders an actions region and an expanded children region', () => {
    render(
      <NotificationContent
        status="error"
        title="2 extensions failed to load"
        actions={<button>Copy error</button>}
      >
        <div>expanded detail row</div>
      </NotificationContent>
    );
    expect(screen.getByRole('button', { name: 'Copy error' })).toBeInTheDocument();
    expect(screen.getByText('expanded detail row')).toBeInTheDocument();
  });
});

describe('NotificationSurface (inline banner shell)', () => {
  it('keeps the surface neutral — status is expressed on the chip, not the surface background', () => {
    const { container } = render(
      <NotificationSurface status="error" title="Backend disconnected" />
    );
    const surface = container.querySelector('[data-testid="notification-surface"]')!;
    // The card ground stays neutral; no status fill leaks onto the whole surface.
    // In place that neutral ground is the subtle fill with a transparent edge and
    // no outline (R-27), and it steps a control inside back to the page ground.
    expect(surface.className).toContain('bg-background-fill');
    expect(surface.className).toContain('border-transparent');
    expect(surface.className).not.toContain('border-border-subtle');
    expect(surface.className).toContain('br-fill-surface');
    expect(surface.className).not.toContain('bg-background-danger');
    // The status hue lives on the chip.
    expect(container.querySelector('[data-status="error"]')).toBeInTheDocument();
  });

  it('renders a dismiss control only when onClose is supplied, and reserves the close gutter once', () => {
    const onClose = vi.fn();
    const { container, rerender } = render(
      <NotificationSurface status="info" title="Heads up" onClose={onClose} />
    );
    const surface = container.querySelector('[data-testid="notification-surface"]')!;
    // Gutter reserved on the container so text/actions can never run under the ×.
    expect(surface.className).toContain('pr-10');
    const dismiss = screen.getByRole('button', { name: /dismiss/i });
    fireEvent.click(dismiss);
    expect(onClose).toHaveBeenCalledTimes(1);

    rerender(<NotificationSurface status="info" title="Heads up" />);
    expect(screen.queryByRole('button', { name: /dismiss/i })).not.toBeInTheDocument();
  });

  it('is flat (no elevation) by default and elevates only when asked', () => {
    const { container, rerender } = render(<NotificationSurface status="info" title="x" />);
    let surface = container.querySelector('[data-testid="notification-surface"]')!;
    expect(surface.className).not.toContain('shadow-popover');
    expect(surface.className).not.toContain('biorouter-popover-surface');
    rerender(<NotificationSurface status="info" title="x" elevated />);
    surface = container.querySelector('[data-testid="notification-surface"]')!;
    // Floating: the one floating-surface recipe on the page ground, not the fill.
    expect(surface.className).toContain('shadow-popover');
    expect(surface.className).toContain('biorouter-popover-surface');
    expect(surface.className).toContain('bg-background-default');
    expect(surface.className).not.toContain('bg-background-fill');
  });
});
