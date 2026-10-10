import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { ThemeProvider, THEME_FAMILIES } from '../../contexts/ThemeContext';
import ThemeFamilySelector from './ThemeFamilySelector';

/**
 * The selector is the only way a user reaches a theme family, so a family that
 * exists in `THEME_FAMILIES` but is missing here is invisible and effectively
 * unshipped. These tests derive from the registry rather than hardcoding names,
 * so adding a family without adding its button fails here.
 */
function renderSelector() {
  return render(
    <ThemeProvider>
      <ThemeFamilySelector />
    </ThemeProvider>
  );
}

describe('ThemeFamilySelector', () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute('data-theme');
  });

  it('renders a button for every registered theme family', () => {
    renderSelector();
    for (const family of THEME_FAMILIES) {
      expect(
        screen.getByTestId(`theme-family-${family}-button`),
        `no button for "${family}" — the family is unreachable from the UI`
      ).toBeInTheDocument();
    }
    // and nothing beyond the registry
    expect(screen.getAllByTestId(/^theme-family-.*-button$/)).toHaveLength(THEME_FAMILIES.length);
  });

  it('includes Roche Limit and labels it', () => {
    renderSelector();
    expect(screen.getByTestId('theme-family-roche-limit-button')).toHaveTextContent('Roche Limit');
  });

  // A segment is a radio: clicking the one already selected changes nothing, so each case
  // first moves away to another family and then selects the one under test.
  it.each(THEME_FAMILIES)('selecting %s writes data-theme and persists it', (family) => {
    renderSelector();
    const other = THEME_FAMILIES.find((candidate) => candidate !== family)!;
    fireEvent.click(screen.getByTestId(`theme-family-${other}-button`));
    fireEvent.click(screen.getByTestId(`theme-family-${family}-button`));
    expect(document.documentElement.getAttribute('data-theme')).toBe(family);
    expect(localStorage.getItem('theme_family')).toBe(family);
    expect(screen.getByTestId(`theme-family-${family}-button`)).toHaveAttribute(
      'aria-checked',
      'true'
    );
  });

  /**
   * One segmented control (spec 2.6): a radiogroup with one radio per family, so the choice is
   * one Tab stop and the arrow keys move it. It used to be a grid of toggle buttons whose column
   * count was a hardcoded class that did not grow with the registry.
   */
  it('is one radiogroup with a radio per family', () => {
    renderSelector();
    const group = screen.getByRole('radiogroup');
    expect(group).toHaveAccessibleName('Color palette');
    expect(screen.getAllByRole('radio')).toHaveLength(THEME_FAMILIES.length);
  });

  it('shows every family’s own swatch, the selected one included', () => {
    renderSelector();
    for (const family of THEME_FAMILIES) {
      const swatch = screen
        .getByTestId(`theme-family-${family}-button`)
        .querySelector<HTMLElement>('.br-swatch-ring');
      expect(swatch?.style.background, family).not.toBe('');
      expect(swatch?.style.background, family).not.toContain('currentcolor');
    }
  });
});
