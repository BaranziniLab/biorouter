import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Select } from './Select';

/**
 * Spec 2.6 / principle 5: the chosen option of a select carries the same trailing 16px check as
 * a selected radio item in every other menu. react-select's own `display: block` rule on an
 * option is unlayered, so the row is made `flex` in the styles API; jsdom cannot measure that,
 * so the flex declaration is asserted at the source.
 */
const OPTIONS = [
  { value: 'daily', label: 'Every day at 9:00' },
  { value: 'weekdays', label: 'Every weekday' },
];

describe('the shared Select’s chosen option', () => {
  it('draws one trailing check on the chosen option and none on the others', () => {
    render(<Select aria-label="Schedule" options={OPTIONS} value={OPTIONS[0]} menuIsOpen />);
    const chosen = screen.getByRole('option', { name: 'Every day at 9:00' });
    const other = screen.getByRole('option', { name: 'Every weekday' });
    expect(chosen).toHaveAttribute('aria-selected', 'true');
    expect(chosen.querySelectorAll('[data-slot="menu-item-indicator"]')).toHaveLength(1);
    expect(other.querySelector('[data-slot="menu-item-indicator"]')).toBeNull();
    // Decorative: the name of the option is its label alone.
    expect(within(chosen).getByText('Every day at 9:00')).toBeInTheDocument();
  });

  it('keeps a caller’s own components, and stands the block rule down', async () => {
    const { readFileSync } = await import('node:fs');
    const { join } = await import('node:path');
    const source = readFileSync(join(__dirname, 'Select.tsx'), 'utf8');
    // Ours first, the caller's last: a call site that passes its own `components` still wins.
    expect(source).toMatch(
      /components=\{\{\s*Option: SelectOption,\s*DropdownIndicator: SelectDropdownIndicator,\s*ClearIndicator: SelectClearIndicator,\s*\.\.\.props\.components,\s*\}\}/
    );
    expect(source).toMatch(/option:\s*\(base\)\s*=>\s*\(\{\s*\.\.\.base,\s*display:\s*'flex'/);
  });
});

/**
 * V4-28 / V3-20: react-select draws its own filled chevron and cross (20px, `fill: currentColor`).
 * Both are the app's 16px stroke-1.5 glyphs now, so the pair is no heavier than any other control.
 */
describe('the shared Select’s indicators', () => {
  it('draws the app’s own chevron and clear mark, not the library’s filled ones', () => {
    const { container } = render(
      <Select
        aria-label="Schedule"
        options={OPTIONS}
        value={OPTIONS[0]}
        isClearable
        classNamePrefix="s"
      />
    );
    const chevron = container.querySelector('.s__dropdown-indicator svg');
    const cross = container.querySelector('.s__clear-indicator svg');
    expect(chevron).not.toBeNull();
    expect(cross).not.toBeNull();
    for (const glyph of [chevron!, cross!]) {
      // The library's own icons carry width/height 20 and no class; ours are 16px by class.
      expect(glyph.getAttribute('width')).not.toBe('20');
      expect(glyph.getAttribute('class')).toMatch(/\bsize-4\b/);
      expect(glyph).toHaveAttribute('aria-hidden', 'true');
    }
    expect(chevron!.getAttribute('class')).toMatch(/chevron-down/);
    expect(cross!.getAttribute('class')).toMatch(/lucide-x\b/);
  });

  it('keeps the clear mark working: a press empties the value', () => {
    const onChange = vi.fn();
    const { container } = render(
      <Select
        aria-label="Schedule"
        options={OPTIONS}
        value={OPTIONS[0]}
        onChange={onChange}
        isClearable
        classNamePrefix="s"
      />
    );
    const clear = container.querySelector('.s__clear-indicator')!;
    fireEvent.mouseDown(clear, { button: 0 });
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange.mock.calls[0][0]).toBeNull();
  });

  it('gives the clear mark the muted ink and a hover to the default ink', async () => {
    const { readFileSync } = await import('node:fs');
    const { join } = await import('node:path');
    const source = readFileSync(join(__dirname, 'Select.tsx'), 'utf8');
    expect(source).toMatch(
      /clearIndicator:\s*\(\)\s*=>\s*'[^']*text-text-muted[^']*hover:text-text-default[^']*'/
    );
  });
});
