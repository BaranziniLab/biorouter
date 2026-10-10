import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import BuiltInBadge from './BuiltInBadge';
import { uiCopy } from './copy';

/**
 * Spec 2.6: the badge is sentence case and its explanation is an InfoTip with the badge as the
 * trigger, so the help is reachable by keyboard and by a screen reader, never a `title=`.
 */
describe('BuiltInBadge', () => {
  it('says Built-in in sentence case, with no caps style', () => {
    render(<BuiltInBadge />);
    const badge = screen.getByText(uiCopy.builtIn);
    expect(badge).not.toHaveClass('text-caps');
    expect(badge).not.toHaveClass('uppercase');
    expect(badge).toHaveClass('text-chip');
    expect(badge).not.toHaveAttribute('title');
  });

  it('carries its help as the badge’s description, and is a tab stop', () => {
    render(<BuiltInBadge />);
    const badge = screen.getByText(uiCopy.builtIn);
    expect(badge).toHaveAttribute('tabindex', '0');
    expect(badge).toHaveAccessibleDescription(uiCopy.builtInHelp);
  });

  it('folds a caller’s two-line help into one sentence line', () => {
    render(<BuiltInBadge title={'Ships with Biorouter.\nRecreated automatically if deleted.'} />);
    expect(screen.getByText(uiCopy.builtIn)).toHaveAccessibleDescription(
      'Ships with Biorouter. Recreated automatically if deleted.'
    );
  });

  it('is plain inside a menu row, where the row is the control', () => {
    render(
      <div role="menu">
        <div role="menuitemcheckbox" aria-checked="false" tabIndex={-1}>
          Developer <BuiltInBadge />
        </div>
      </div>
    );
    const badge = screen.getByText(uiCopy.builtIn);
    expect(badge).not.toHaveAttribute('tabindex');
    expect(badge).not.toHaveAttribute('aria-describedby');
  });
});
