import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
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
    expect(source).toMatch(/components=\{\{ Option: SelectOption, \.\.\.props\.components \}\}/);
    expect(source).toMatch(/option:\s*\(base\)\s*=>\s*\(\{\s*\.\.\.base,\s*display:\s*'flex'/);
  });
});
