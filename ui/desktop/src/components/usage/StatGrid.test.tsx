import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { StatGrid, type StatItem } from './StatGrid';

const items: StatItem[] = [
  { key: 'chats', label: 'Chats', value: '352' },
  {
    key: 'tokens',
    label: 'Tokens',
    value: '87.9B',
    title: '87,912,345,678',
    help: 'Some days lack full token records, so this total is a lower bound.',
  },
  { key: 'active', label: 'Active days', value: '12', suffix: '/30' },
  { key: 'cost', label: 'Est. cost', value: '$12.00', note: 'Known conservative subtotal' },
];

describe('StatGrid', () => {
  it('is a description list of flat tiles: a 12px label over a 24px figure', () => {
    render(<StatGrid items={items} aria-label="Usage figures" />);
    const list = screen.getByLabelText('Usage figures');
    expect(list.tagName).toBe('DL');
    const terms = within(list)
      .getAllByRole('term')
      .map((term) => term.querySelector('.br-stat-label-text')?.textContent);
    expect(terms).toEqual(['Chats', 'Tokens', 'Active days', 'Est. cost']);

    const chats = screen.getByText('352').closest('dd')!;
    expect(chats).toHaveClass('text-title');
    expect(screen.getByText('Chats').closest('dt')).toHaveClass('text-supporting');
    // No tile boxes and no icons.
    expect(list.querySelectorAll('svg')).toHaveLength(list.querySelectorAll('.br-info-tip').length);
    expect(list.querySelector('[class*="border"]')).toBeNull();
  });

  it('puts the exact figure in a hover title and the help behind an InfoTip', () => {
    render(<StatGrid items={items} />);
    expect(screen.getByText('87.9B')).toHaveAttribute('title', '87,912,345,678');
    const tip = screen.getByRole('button', { name: 'About Tokens' });
    expect(tip).toHaveAccessibleDescription(
      'Some days lack full token records, so this total is a lower bound.'
    );
    // Only the item that has help gets a glyph.
    expect(screen.getAllByRole('button')).toHaveLength(1);
  });

  it('draws a suffix on the figure line and a note under it', () => {
    render(<StatGrid items={items} />);
    expect(screen.getByText('/30').closest('dd')).toHaveTextContent('12/30');
    const note = screen.getByText('Known conservative subtotal');
    expect(note.tagName).toBe('DD');
    expect(note).toHaveClass('text-supporting');
  });

  it('mounts at rest, and fades figures in only when the animate key changes', () => {
    const { rerender } = render(<StatGrid items={items} animateKey="1y" />);
    const before = screen.getByText('352').closest('dd')!;
    expect(before).not.toHaveAttribute('data-animate');

    rerender(<StatGrid items={items} animateKey="1y" />);
    expect(screen.getByText('352').closest('dd')).toBe(before);

    rerender(<StatGrid items={[{ ...items[0], value: '20' }]} animateKey="7d" />);
    const after = screen.getByText('20').closest('dd')!;
    expect(after).not.toBe(before);
    expect(after).toHaveAttribute('data-animate');
  });
});
