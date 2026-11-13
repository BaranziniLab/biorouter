import { describe, expect, it } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { ERROR_DETAILS_LABEL, ErrorUI, errorLayers } from './ErrorBoundary';

/**
 * The full-screen fallback (owner, 2026-11-13): the error's first line stays in view, as an error
 * that just happened should, and the whole text folds behind "Show error details" instead of an
 * unbounded block under the Reload button.
 */
describe('ErrorUI', () => {
  it('shows a one-line error whole, in the code face, with nothing to fold', () => {
    render(<ErrorUI error="Cannot read properties of undefined (reading 'id')" />);
    const line = screen.getByTestId('error-first-line');
    expect(line).toHaveTextContent("Cannot read properties of undefined (reading 'id')");
    expect(line).toHaveClass('font-mono', 'text-code');
    expect(screen.queryByRole('button', { name: ERROR_DETAILS_LABEL })).toBeNull();
    expect(screen.getByRole('button', { name: 'Reload' })).toBeInTheDocument();
  });

  it('keeps the first line in view and folds the rest behind Show error details', () => {
    const error = 'TypeError: x is not a function\n    at App (App.tsx:12:3)\n    at render';
    render(<ErrorUI error={error} />);
    expect(screen.getByTestId('error-first-line')).toHaveTextContent(
      /^TypeError: x is not a function$/
    );
    expect(screen.queryByText(/at App \(App\.tsx/)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: ERROR_DETAILS_LABEL }));
    const full = screen.getByText(/at App \(App\.tsx/);
    expect(full.tagName).toBe('PRE');
    expect(full).toHaveClass('font-mono', 'text-code');
  });

  it('folds a single line long enough to be clamped', () => {
    expect(errorLayers('short')).toEqual({ line: 'short', more: false });
    expect(errorLayers('x'.repeat(201)).more).toBe(true);
  });
});
