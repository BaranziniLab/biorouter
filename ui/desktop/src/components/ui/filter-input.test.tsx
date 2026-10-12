import { useState } from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import { FilterInput } from './filter-input';
import { uiCopy } from './copy';

function View({ findShortcut, placeholder }: { findShortcut?: boolean; placeholder?: string }) {
  const [value, setValue] = useState('');
  return (
    <div>
      <button type="button">Elsewhere</button>
      <FilterInput
        value={value}
        onValueChange={setValue}
        findShortcut={findShortcut}
        placeholder={placeholder}
      />
    </div>
  );
}

const field = () => screen.getByRole('searchbox', { name: 'Filter' });

describe('FilterInput', () => {
  it('is a named, no-drag search field with the Filter placeholder and a search glyph', () => {
    render(<View />);
    expect(field()).toHaveAttribute('placeholder', uiCopy.filter);
    expect(field()).toHaveClass('br-filter-input-field');
    expect(field().closest('.br-filter-input')).toHaveClass('no-drag');
    expect(document.querySelector('.br-filter-input-icon')).not.toBeNull();
  });

  it('takes History’s placeholder', () => {
    render(<View placeholder={uiCopy.searchHistory} />);
    expect(field()).toHaveAttribute('placeholder', 'Search history');
  });

  it('focuses on ⌘F / Ctrl+F while mounted', () => {
    render(<View />);
    screen.getByRole('button', { name: 'Elsewhere' }).focus();
    fireEvent.keyDown(window, { key: 'f', metaKey: true, ctrlKey: true });
    // One of the two modifiers is this platform's; try both so the test is platform-blind.
    if (document.activeElement !== field()) {
      fireEvent.keyDown(window, { key: 'f', metaKey: true });
      if (document.activeElement !== field())
        fireEvent.keyDown(window, { key: 'f', ctrlKey: true });
    }
    expect(field()).toHaveFocus();
  });

  it('leaves ⌘F alone when the shortcut is off', () => {
    render(<View findShortcut={false} />);
    fireEvent.keyDown(window, { key: 'f', metaKey: true });
    fireEvent.keyDown(window, { key: 'f', ctrlKey: true });
    expect(field()).not.toHaveFocus();
  });

  it('clears on Escape, then leaves the field on a second Escape', async () => {
    const user = userEvent.setup();
    render(<View />);
    await user.click(field());
    await user.keyboard('skill');
    expect(field()).toHaveValue('skill');
    await user.keyboard('{Escape}');
    expect(field()).toHaveValue('');
    expect(field()).toHaveFocus();
    await user.keyboard('{Escape}');
    expect(field()).not.toHaveFocus();
  });

  it('spells the tooltip with the platform’s shortcut', () => {
    expect(uiCopy.filterTooltip('Filter', true)).toBe('Filter · ⌘F');
    expect(uiCopy.filterTooltip('Filter', false)).toBe('Filter · Ctrl+F');
  });
});
