import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { PageHeader, PageHeaderAction } from './PageHeader';
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

describe('PageHeaderAction', () => {
  it('names the icon button by its label, without a title attribute', () => {
    render(<PageHeaderAction icon={Upload} label="Import workflow" onClick={() => {}} />);

    const button = screen.getByRole('button', { name: 'Import workflow' });
    expect(button).not.toHaveAttribute('title');
  });
});
