import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import ParameterInput from './ParameterInput';

afterEach(cleanup);

const parameter = {
  key: 'cohort',
  description: 'Which cohort',
  input_type: 'string' as const,
  requirement: 'required' as const,
};

/**
 * A workflow parameter is a filled card (owner message 13, R-27): the subtle fill, a transparent
 * 1px edge and no outline, a step back to the page ground for the fields inside, and no rule
 * between its header and its details. jsdom runs no Tailwind, so these are the classes the
 * paint is made of; `Layout/viewsFill.browser.test.tsx` measures the paint.
 */
describe('ParameterInput card', () => {
  it('is the subtle fill with a transparent edge, and a filled surface for its children', () => {
    const { container } = render(<ParameterInput parameter={parameter} onChange={vi.fn()} />);

    const card = container.querySelector('.parameter-input')!;
    expect(card).toHaveClass('bg-background-fill', 'border', 'border-transparent');
    expect(card).toHaveClass('br-fill-surface');
    expect(card.className).not.toMatch(/border-border-/);
  });

  it('puts no rule between the header and the details', () => {
    const { container } = render(<ParameterInput parameter={parameter} onChange={vi.fn()} />);

    const details = container.querySelector('.parameter-input')!.lastElementChild!;
    expect(details.className).not.toMatch(/\bborder-t\b/);
  });

  it('steps its header to the strong fill under the pointer, not an ink wash on a fill', () => {
    const { container } = render(
      <ParameterInput parameter={parameter} onChange={vi.fn()} onToggleExpanded={vi.fn()} />
    );

    const header = container.querySelector('.parameter-input')!.firstElementChild!;
    expect(header).toHaveClass('hover:bg-background-fill-strong');
    expect(header.className).not.toMatch(/overlay-hover/);
  });

  it('draws the {{key}} chip on the page ground, since the muted ground IS the card’s fill', () => {
    render(<ParameterInput parameter={parameter} onChange={vi.fn()} />);

    expect(screen.getByText('cohort').closest('code')).toHaveClass('bg-background-default');
  });

  it('draws its two choices as the app’s native select on the field skin', () => {
    render(<ParameterInput parameter={parameter} onChange={vi.fn()} />);

    for (const name of ['Input type', 'Requirement']) {
      const select = screen.getByLabelText(name);
      expect(select.tagName).toBe('SELECT');
      expect(select).toHaveClass('br-field', 'appearance-none');
      expect(select.className).not.toMatch(/\bbg-/);
    }
  });
});
