import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../../api';
import { TooltipProvider } from '../../../ui/Tooltip';
import { ProviderCard } from './ProviderCard';

function provider(configured: boolean): ProviderDetails {
  return {
    name: 'versa_azure',
    is_configured: configured,
    metadata: {
      name: 'versa_azure',
      display_name: 'Versa API Azure',
      description: 'A description long enough that the row truncates it in a narrow window',
      default_model: 'gpt-5.5',
      known_models: [],
      model_doc_link: '',
      config_keys: [],
    },
  } as unknown as ProviderDetails;
}

function renderRow(configured: boolean, onConfigure = vi.fn()) {
  render(
    <TooltipProvider>
      <ProviderCard
        provider={provider(configured)}
        onConfigure={onConfigure}
        onLaunch={vi.fn()}
        isOnboarding={false}
      />
    </TooltipProvider>
  );
  return onConfigure;
}

// W2-PRV-4: the Configure button sat at opacity 0 until hovered, so a keyboard
// user tabbed onto an invisible button; and the truncated description's tooltip
// hung off a <p> nothing could focus.
describe('ProviderCard keyboard access', () => {
  it('reveals the hover-only actions while anything in the row has focus', () => {
    renderRow(true);
    const actions = screen.getByTestId('provider-actions-versa_azure');
    expect(actions).toHaveClass('opacity-0', 'group-hover:opacity-100');
    // jsdom computes no Tailwind, so the reveal is asserted as the class the
    // compiled stylesheet already carries elsewhere in the app.
    expect(actions).toHaveClass('group-focus-within:opacity-100');
    expect(screen.getByTestId('provider-card-versa_azure')).toHaveClass('group');
  });

  it('reaches the description and then Configure by Tab, and Enter opens the dialog', async () => {
    const user = userEvent.setup();
    const onConfigure = renderRow(true);

    await user.tab();
    expect(screen.getByTestId('provider-description-versa_azure')).toHaveFocus();

    await user.tab();
    const configure = screen.getByRole('button');
    expect(configure).toHaveFocus();
    expect(screen.getByTestId('provider-actions-versa_azure')).toContainElement(configure);

    await user.keyboard('{Enter}');
    expect(onConfigure).toHaveBeenCalledTimes(1);
  });

  it('opens the full description in a tooltip on keyboard focus', async () => {
    const user = userEvent.setup();
    renderRow(false);
    await user.tab();
    expect(screen.getByTestId('provider-description-versa_azure')).toHaveFocus();
    expect(
      await screen.findByRole('tooltip', {
        name: /A description long enough that the row truncates it/,
      })
    ).toBeInTheDocument();
  });
});
