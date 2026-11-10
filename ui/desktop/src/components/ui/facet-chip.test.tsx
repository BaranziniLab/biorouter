import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { createRef } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { FACET_CHIP_CLASS_NAME, FacetChip } from './facet-chip';
import { Popover, PopoverContent, PopoverTrigger } from './popover';

describe('FacetChip', () => {
  it('wears the facet strip class string exactly', () => {
    expect(FACET_CHIP_CLASS_NAME).toBe(
      'br-facet biorouter-focus-surface inline-flex h-control-md flex-none items-center gap-2 rounded-element px-2.5 text-label transition-[color,background-color,border-color]'
    );
    render(<FacetChip>Source</FacetChip>);
    const chip = screen.getByRole('button', { name: 'Source' });
    expect(chip.className).toBe(FACET_CHIP_CLASS_NAME);
    expect(chip.getAttribute('type')).toBe('button');
  });

  it('matches the class string the Knowledge graph strip writes', () => {
    // Until the strip renders FacetChip itself, its literal string must stay this one.
    const strip = readFileSync(
      join(__dirname, '..', 'knowledge', 'graph', 'GraphFacetStrip.tsx'),
      'utf8'
    );
    const usesChip = /<FacetChip\b/.test(strip);
    if (!usesChip) expect(strip).toContain(FACET_CHIP_CLASS_NAME);
    else expect(strip).not.toMatch(/className=\{?[`'"]br-facet biorouter-focus-surface/);
  });

  it('appends a caller class', () => {
    render(<FacetChip className="br-facet-legend ml-auto">Legend</FacetChip>);
    expect(screen.getByRole('button').className).toBe(
      `${FACET_CHIP_CLASS_NAME} br-facet-legend ml-auto`
    );
  });

  it('writes data-engaged from engaged', () => {
    const { rerender } = render(<FacetChip engaged>Outcome</FacetChip>);
    const chip = () => screen.getByRole('button');
    expect(chip().getAttribute('data-engaged')).toBe('true');
    rerender(<FacetChip engaged={false}>Outcome</FacetChip>);
    expect(chip().getAttribute('data-engaged')).toBe('false');
    rerender(<FacetChip>Outcome</FacetChip>);
    expect(chip().hasAttribute('data-engaged')).toBe(false);
  });

  it('forwards its ref and native props', async () => {
    const ref = createRef<HTMLButtonElement>();
    const onClick = vi.fn();
    render(
      <FacetChip ref={ref} aria-label="Filters, set: monitor" onClick={onClick}>
        Filters: monitor
      </FacetChip>
    );
    expect(ref.current).toBe(screen.getByRole('button', { name: 'Filters, set: monitor' }));
    await userEvent.click(ref.current!);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('works as an asChild trigger', async () => {
    render(
      <Popover>
        <PopoverTrigger asChild>
          <FacetChip engaged>Filters</FacetChip>
        </PopoverTrigger>
        <PopoverContent>Inside</PopoverContent>
      </Popover>
    );
    const chip = screen.getByRole('button', { name: 'Filters' });
    expect(chip.className).toBe(FACET_CHIP_CLASS_NAME);
    expect(chip.getAttribute('aria-expanded')).toBe('false');
    await userEvent.click(chip);
    expect(chip.getAttribute('aria-expanded')).toBe('true');
    expect(chip.getAttribute('data-engaged')).toBe('true');
    expect(await screen.findByText('Inside')).toBeInTheDocument();
  });
});
