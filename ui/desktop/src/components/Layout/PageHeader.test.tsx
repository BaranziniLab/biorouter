import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { PageHeader } from './PageHeader';

/**
 * The header's CONTRACT, not its paint. jsdom runs no Tailwind, so nothing here
 * asserts a colour or a computed width — `styles/measures.test.ts` makes the
 * measure claims against the stylesheet and the source, and the visual pass is
 * in the PR. What a render test can prove is the STRUCTURE the other two rely
 * on: that the actions land in the strip rather than beside the title, and that
 * the hairline is outside the column rather than on it.
 */
describe('PageHeader', () => {
  it('renders the title as the page heading, with its description', () => {
    render(<PageHeader title="Workflows" description="View and manage your saved workflows." />);

    expect(screen.getByRole('heading', { level: 1, name: 'Workflows' })).toBeInTheDocument();
    expect(screen.getByText('View and manage your saved workflows.')).toBeInTheDocument();
  });

  it('omits the description paragraph entirely when there is none', () => {
    const { container } = render(<PageHeader title="Extensions" />);

    expect(container.querySelectorAll('p')).toHaveLength(0);
  });

  /**
   * The operator's decision, pinned: the actions are NOT on the title row. A
   * test that only asserted "the button renders" would pass in both placements,
   * which is exactly the drift this component was built to end — so this asserts
   * where the button lands, and the next test asserts where it does not.
   */
  it('puts the actions in a control strip below the description', () => {
    const { container } = render(
      <PageHeader
        title="Skills"
        description="Add and manage skills."
        actions={<button type="button">Add skill</button>}
      />
    );

    const strip = container.querySelector('.biorouter-settings-control-strip');
    expect(strip).not.toBeNull();
    expect(strip).toContainElement(screen.getByRole('button', { name: 'Add skill' }));
    expect(strip).toHaveClass('mt-5');
  });

  it('never places an action inside the title row', () => {
    render(<PageHeader title="Scheduler" actions={<button type="button">New schedule</button>} />);

    const heading = screen.getByRole('heading', { level: 1, name: 'Scheduler' });
    const titleRow = heading.parentElement;
    expect(titleRow).not.toBeNull();
    expect(titleRow?.querySelector('button')).toBeNull();
  });

  it('renders no strip at all when a view has no actions', () => {
    const { container } = render(<PageHeader title="Settings" description="Manage models." />);

    expect(container.querySelector('.biorouter-settings-control-strip')).toBeNull();
  });

  it('puts a title adornment on the title row and extra children below', () => {
    const { container } = render(
      <PageHeader
        title="Chat history"
        description="View and search your past chats."
        titleAdornment={<span data-testid="adornment">12</span>}
      >
        <label>
          <input type="checkbox" />
          Show subagent runs
        </label>
      </PageHeader>
    );

    const heading = screen.getByRole('heading', { level: 1, name: 'Chat history' });
    expect(heading.parentElement).toContainElement(screen.getByTestId('adornment'));

    // The checkbox is a sibling of the description, not of the title.
    const checkbox = screen.getByRole('checkbox');
    expect(heading.parentElement).not.toContainElement(checkbox);
    expect(container.firstElementChild).toContainElement(checkbox);
  });

  it('sizes its reading column to the chat measure', () => {
    const { container } = render(<PageHeader title="Extensions" />);

    const column = container.querySelector('.biorouter-readable-content');
    expect(column).not.toBeNull();
    expect(column).toHaveAttribute('data-size', 'chat');
  });

  /**
   * The hairline is on the WRAPPER, not on the column — the one difference
   * between the seven views that got it right and Skills, whose rule stopped at
   * the reading measure while every sibling's ran edge to edge. Asserted as
   * "the column does not carry it AND its parent does", because either half
   * alone passes while the bug is present.
   */
  it('hangs the hairline outside the reading column so the rule is full-bleed', () => {
    const { container } = render(<PageHeader title="Built apps" />);

    const wrapper = container.firstElementChild as HTMLElement;
    const column = container.querySelector('.biorouter-readable-content') as HTMLElement;

    expect(wrapper).toHaveClass('biorouter-page-header');
    expect(column).not.toHaveClass('biorouter-page-header');
    expect(column).not.toHaveClass('border-b');
    expect(column.parentElement).toBe(wrapper);
  });

  /**
   * `page-transition` matches no CSS rule in this repo and, measured in the
   * running app, produces no animation and no transition. It was on seven of
   * the eight headers this component replaces. Pinned so a later "restore the
   * animation" edit has to add the missing stylesheet rule rather than
   * re-adding the no-op class.
   */
  it('carries no page-transition class', () => {
    const { container } = render(<PageHeader title="Workflows" description="Anything." />);

    expect(container.querySelector('.page-transition')).toBeNull();
  });
});
