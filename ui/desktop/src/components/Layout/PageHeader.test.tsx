import { act, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PageHeader, PageHeaderAction, PageHeaderMenu } from './PageHeader';
import { Upload } from '../icons/app-icons';

/**
 * The band's CONTRACT, not its paint. jsdom runs no Tailwind and lays nothing out, so nothing
 * here asserts a height or a position: `pageBandGeometry.browser.test.tsx` measures the band in
 * a real layout engine against the authored CSS. What a render test can prove is the STRUCTURE
 * the geometry and the drag rects rely on.
 */
describe('PageHeader', () => {
  it('renders the title as the one page heading', () => {
    render(<PageHeader title="Workflows" />);

    expect(screen.getByRole('heading', { level: 1, name: 'Workflows' })).toBeInTheDocument();
  });

  /**
   * The title names its own ink. Settings renders the band inside a Tabs root that is
   * `text-text-muted`, so an inherited ink painted "Settings" like the inactive tabs beside it
   * (vision round 1, V4-06). jsdom runs no Tailwind, so the contract is the class on the h1
   * itself: an ancestor's colour cannot beat a utility on the element.
   */
  it('gives the title its own default ink so a muted ancestor cannot recolour it', () => {
    render(
      <div className="text-text-muted">
        <PageHeader title="Settings" />
      </div>
    );

    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toHaveClass(
      'text-text-default'
    );
  });

  /**
   * Principle 2: the explanation is help, not a paragraph. It must still reach a screen reader
   * without a hover, so the info button is described by a node that is always in the DOM.
   */
  it('renders info as an About button described by the help, never as a paragraph', () => {
    const { container } = render(
      <PageHeader title="Workflows" info="Reusable chat setups. Start one from here." />
    );

    const about = screen.getByRole('button', { name: 'About Workflows' });
    expect(about).toHaveAccessibleDescription('Reusable chat setups. Start one from here.');
    expect(container.querySelectorAll('p')).toHaveLength(0);
  });

  it('treats the deprecated description as info', () => {
    render(<PageHeader title="Skills" description="Reusable instruction sets." />);

    expect(screen.getByRole('button', { name: 'About Skills' })).toHaveAccessibleDescription(
      'Reusable instruction sets.'
    );
  });

  it('renders no help button when there is nothing to explain', () => {
    render(<PageHeader title="Extensions" />);

    expect(screen.queryByRole('button', { name: /^About/ })).toBeNull();
  });

  /**
   * The band: one line, the actions at its trailing edge. This is the reversal of the
   * 2026-09-07 "actions on their own line" decision, pinned in its new direction.
   */
  it('puts the title, the adornment and the actions on one band', () => {
    render(
      <PageHeader
        title="Scheduler"
        adornment={<span data-testid="adornment">3</span>}
        actions={<button type="button">New schedule</button>}
      />
    );

    const band = screen.getByTestId('page-header');
    expect(band.tagName).toBe('HEADER');
    expect(band).toHaveAttribute('data-band');
    expect(band).toContainElement(screen.getByRole('heading', { level: 1, name: 'Scheduler' }));
    expect(band).toContainElement(screen.getByTestId('adornment'));
    expect(band).toContainElement(screen.getByRole('button', { name: 'New schedule' }));
    expect(band.querySelector('.biorouter-settings-control-strip')).toBeNull();
  });

  it('keeps the deprecated titleAdornment working as the adornment', () => {
    render(
      <PageHeader title="Chat history" titleAdornment={<span data-testid="adornment">12</span>} />
    );

    expect(screen.getByTestId('page-header')).toContainElement(screen.getByTestId('adornment'));
  });

  /**
   * The drag geometry (issue #74): the header itself declares nothing; every control lives in
   * the inner bar, which is the drag rect and which takes the titlebar reserve as a margin.
   * Asserted as structure because jsdom has no drag rects.
   */
  it('holds every control inside the drag bar, which is the header’s only child', () => {
    render(
      <PageHeader
        title="Scheduler"
        onBack={() => {}}
        info="Help."
        tabs={<div role="tablist" />}
        actions={<button type="button">Run now</button>}
      />
    );

    const band = screen.getByTestId('page-header');
    expect(band.children).toHaveLength(1);
    const bar = band.firstElementChild as HTMLElement;
    expect(bar).toHaveClass('biorouter-page-header-bar');
    for (const button of screen.getAllByRole('button')) expect(bar).toContainElement(button);
    expect(bar).toContainElement(screen.getByRole('tablist'));
  });

  /**
   * The band is a recessed ground: its paint is the sidebar's, on which the rest fill is 1.03:1
   * off, so the controls in it rest one step deeper. It says so with the shared class and holds no
   * ground of its own (`viewsFill.browser.test.tsx` measures the paint, and proves it goes without
   * the class). jsdom runs no CSS, so the contract here is the class on the `<header>` itself.
   */
  it('wears the shared recessed-ground class on the header, so its controls step one fill in', () => {
    render(<PageHeader title="Workflows" actions={<button type="button">Sort</button>} />);

    expect(screen.getByTestId('page-header')).toHaveClass('br-ground-recessed');
  });

  it('renders a back button before the title when the page is a drill-in', async () => {
    const onBack = vi.fn();
    render(<PageHeader title="nightly-cohort" onBack={onBack} backLabel="Back to Scheduler" />);

    const back = screen.getByRole('button', { name: 'Back to Scheduler' });
    const heading = screen.getByRole('heading', { level: 1 });
    expect(back.compareDocumentPosition(heading) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    await userEvent.click(back);
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it('renders no back button on a top-level page', () => {
    render(<PageHeader title="Workflows" />);

    expect(screen.queryByRole('button', { name: 'Back' })).toBeNull();
  });

  it('renders tabs inline in the band', () => {
    render(
      <PageHeader
        title="Settings"
        tabs={
          <div role="tablist">
            <button role="tab" type="button">
              App
            </button>
          </div>
        }
      />
    );

    expect(screen.getByTestId('page-header')).toContainElement(screen.getByRole('tab'));
  });

  /** The band never sits in the reading column; only a view's body does. */
  it('does not put the band in a reading column', () => {
    const { container } = render(<PageHeader title="Built apps" />);

    expect(container.querySelector('.biorouter-readable-content')).toBeNull();
    expect(screen.getByTestId('page-header')).toHaveClass('biorouter-page-header');
  });

  it('renders the deprecated children in a row under the band, not in it', () => {
    render(
      <PageHeader title="Chat history">
        <label>
          <input type="checkbox" />
          Show subagent runs
        </label>
      </PageHeader>
    );

    const checkbox = screen.getByRole('checkbox');
    expect(screen.getByTestId('page-header')).not.toContainElement(checkbox);
    expect(checkbox.closest('.biorouter-page-subband')).not.toBeNull();
    expect(checkbox.closest('.biorouter-readable-content')).toHaveAttribute('data-size', 'chat');
  });

  it('carries no page-transition class', () => {
    const { container } = render(<PageHeader title="Workflows" info="Anything." />);

    expect(container.querySelector('.page-transition')).toBeNull();
  });
});

/**
 * The mouse's Back key leaves a drill-in the way its Back button does (V4-14: `BackButton` was the
 * only place the key lived, so a band's `onBack`, which is a plain icon button, lost it). The
 * band calls the hook itself, so every page gets the key from one place.
 */
describe('PageHeader mouse Back', () => {
  let dispose: ReturnType<typeof vi.fn>;
  let on: ReturnType<typeof vi.fn>;
  let command: (() => void) | undefined;

  beforeEach(() => {
    dispose = vi.fn();
    command = undefined;
    on = vi.fn((_channel: string, listener: () => void) => {
      command = listener;
      return dispose;
    });
    Object.defineProperty(window, 'electron', {
      configurable: true,
      writable: true,
      value: { on },
    });
  });

  afterEach(() => {
    Object.defineProperty(window, 'electron', {
      configurable: true,
      writable: true,
      value: undefined,
    });
  });

  it('leaves on the main process command and on the mouse’s Back key, as the button does', () => {
    const onBack = vi.fn();
    render(<PageHeader title="nightly-cohort" onBack={onBack} />);

    expect(on).toHaveBeenCalledWith('mouse-back-button-clicked', expect.any(Function));
    act(() => command?.());
    fireEvent.mouseUp(document, { button: 3 });
    expect(onBack).toHaveBeenCalledTimes(2);
  });

  it('listens to nothing on a top-level page, which has no Back', () => {
    render(<PageHeader title="Workflows" />);

    expect(on).not.toHaveBeenCalled();
  });

  it('keeps one pair of listeners and calls the LATEST handler when the caller passes a new arrow each render', () => {
    const first = vi.fn();
    const second = vi.fn();
    const { rerender } = render(<PageHeader title="a" onBack={() => first()} />);
    rerender(<PageHeader title="a" onBack={() => second()} />);
    rerender(<PageHeader title="a" onBack={() => second()} />);

    expect(on).toHaveBeenCalledTimes(1);
    expect(dispose).not.toHaveBeenCalled();
    fireEvent.mouseUp(document, { button: 3 });
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);
  });

  it('lets go of the key when the page unmounts', () => {
    const onBack = vi.fn();
    const { unmount } = render(<PageHeader title="a" onBack={onBack} />);
    unmount();

    expect(dispose).toHaveBeenCalledTimes(1);
    fireEvent.mouseUp(document, { button: 3 });
    expect(onBack).not.toHaveBeenCalled();
  });
});

describe('PageHeaderAction', () => {
  it('names the icon button by its label, without a title attribute', () => {
    render(<PageHeaderAction icon={Upload} label="Import workflow" onClick={() => {}} />);

    const button = screen.getByRole('button', { name: 'Import workflow' });
    expect(button).not.toHaveAttribute('title');
  });

  /**
   * `busy` is "a request this button started is in flight" (V3-R2-1). A focused button that
   * turns `disabled` hands focus to <body> in Chromium, so the action stays a live control that
   * says it is working and ignores the press. jsdom never moves focus off a disabled button, so
   * the real-engine half is `schedule/scheduleFocus.browser.test.tsx`.
   */
  describe('busy', () => {
    it('stays enabled and focusable, says it is working, and ignores a press', async () => {
      const onClick = vi.fn();
      const user = userEvent.setup();
      render(<PageHeaderAction icon={Upload} label="Refresh" onClick={onClick} busy />);

      const button = screen.getByRole('button', { name: 'Refresh' });
      expect(button).not.toHaveAttribute('disabled');
      expect(button).toHaveAttribute('aria-disabled', 'true');
      expect(button).toHaveAttribute('aria-busy', 'true');
      expect(button).toHaveClass('br-action-busy');

      button.focus();
      await user.keyboard('{Enter}');
      await user.click(button);
      expect(button).toHaveFocus();
      expect(onClick).not.toHaveBeenCalled();
    });

    it('presses normally, with none of that, when it is not busy', async () => {
      const onClick = vi.fn();
      const user = userEvent.setup();
      render(<PageHeaderAction icon={Upload} label="Refresh" onClick={onClick} busy={false} />);

      const button = screen.getByRole('button', { name: 'Refresh' });
      expect(button).not.toHaveAttribute('aria-disabled');
      expect(button).not.toHaveAttribute('aria-busy');
      expect(button).not.toHaveClass('br-action-busy');
      await user.click(button);
      expect(onClick).toHaveBeenCalledTimes(1);
    });

    it('leaves a caller’s own aria-disabled and class alone when it is not busy', () => {
      render(
        <PageHeaderAction
          icon={Upload}
          label="Share"
          aria-disabled
          className="br-transcript-action"
          onClick={() => {}}
        />
      );

      const button = screen.getByRole('button', { name: 'Share' });
      expect(button).toHaveAttribute('aria-disabled', 'true');
      expect(button).toHaveClass('br-transcript-action');
      expect(button).not.toHaveClass('br-action-busy');
    });

    it('keeps a caller’s class while busy', () => {
      render(
        <PageHeaderAction
          icon={Upload}
          label="Share"
          className="br-transcript-action"
          onClick={() => {}}
          busy
        />
      );

      expect(screen.getByRole('button', { name: 'Share' })).toHaveClass(
        'br-transcript-action',
        'br-action-busy'
      );
    });
  });
});

describe('PageHeaderMenu', () => {
  it('opens the row-menu items from a More actions button', async () => {
    const onEdit = vi.fn();
    render(
      <PageHeaderMenu
        items={[
          { label: 'Edit', onSelect: onEdit },
          { kind: 'separator' },
          { label: 'Delete', onSelect: () => {}, destructive: true },
        ]}
      />
    );

    // Radix opens a dropdown on pointerdown, not click.
    fireEvent.pointerDown(screen.getByRole('button', { name: 'More actions' }), {
      button: 0,
      ctrlKey: false,
    });
    expect(await screen.findByRole('menuitem', { name: 'Delete' })).toHaveAttribute(
      'data-variant',
      'destructive'
    );
    await userEvent.click(screen.getByRole('menuitem', { name: 'Edit' }));
    expect(onEdit).toHaveBeenCalledTimes(1);
  });
});
