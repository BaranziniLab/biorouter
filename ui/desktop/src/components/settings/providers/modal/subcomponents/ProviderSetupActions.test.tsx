import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import ProviderSetupActions from './ProviderSetupActions';

/**
 * Vision V4-R7-6: a bare `<strong>` takes Tailwind preflight's `font-weight: bolder`, which
 * computes to 700, a weight the type system does not allow (400 and 600 only). The provider's
 * name in the two removal sentences is emphasised at 600. Arial draws 600 as bold, so nothing
 * moves on screen; the computed weight is the whole defect, and the class on each run is what
 * sets it (the sweep's earlier commits read the engine once for the pattern, in
 * `settings/privacy/privacyPanelEmphasisWeight.browser.test.tsx`).
 */

const NO_HEAVIER = /\bfont-(bold|extrabold|black)\b/;

function emphasised(name: string): HTMLElement {
  const strong = screen.getByText(name);
  expect(strong.tagName).toBe('STRONG');
  return strong;
}

describe('ProviderSetupActions: the provider name is emphasised at 600 (V4-R7-6)', () => {
  it('names the provider in use at 600 when it cannot be removed', () => {
    render(
      <ProviderSetupActions
        onCancel={vi.fn()}
        onSubmit={vi.fn()}
        showDeleteConfirmation
        isActiveProvider
        providerName="Anthropic"
      />
    );
    const strong = emphasised('Anthropic');
    expect(strong).toHaveClass('font-semibold');
    expect(strong.className).not.toMatch(NO_HEAVIER);
  });

  it('names the provider whose configuration is about to be deleted at 600, in the default ink', () => {
    render(
      <ProviderSetupActions
        onCancel={vi.fn()}
        onSubmit={vi.fn()}
        showDeleteConfirmation
        providerName="Anthropic"
      />
    );
    const strong = emphasised('Anthropic');
    expect(strong).toHaveClass('font-semibold');
    expect(strong).toHaveClass('text-text-default');
    expect(strong.className).not.toMatch(NO_HEAVIER);
  });
});
