import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../../api';
import { TooltipProvider } from '../../../ui/Tooltip';
import { ProviderCard } from './ProviderCard';
import { catalogCopy } from '../copy';

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
    // `.br-row-actions` is the shared reveal (main.css): opacity only, on the settings row's
    // hover AND focus-within, and always shown without a hover pointer. jsdom computes no CSS,
    // so what is asserted is the pair of classes that carry it.
    expect(actions).toHaveClass('br-row-actions');
    expect(screen.getByTestId('provider-card-versa_azure')).toHaveClass('biorouter-settings-row');
  });

  it('shows the actions at all times during first run', () => {
    render(
      <TooltipProvider>
        <ProviderCard
          provider={provider(true)}
          onConfigure={vi.fn()}
          onLaunch={vi.fn()}
          isOnboarding
        />
      </TooltipProvider>
    );
    expect(screen.getByTestId('provider-actions-versa_azure')).not.toHaveClass('br-row-actions');
  });

  it('is text: no initial tile, and "Configured" is a success dot and a word', () => {
    const { container } = render(
      <TooltipProvider>
        <ProviderCard
          provider={provider(true)}
          onConfigure={vi.fn()}
          onLaunch={vi.fn()}
          isOnboarding={false}
        />
      </TooltipProvider>
    );
    expect(screen.queryByText('V')).toBeNull();
    const status = screen.getByText(catalogCopy.configured);
    expect(status.querySelector('[data-slot="status-dot"][data-tone="success"]')).not.toBeNull();
    // The dot is decoration; the word is the status.
    expect(container.querySelector('[data-slot="status-dot"]')).toHaveAttribute('aria-hidden');
  });

  it('names its Configure button for the row, so twenty rows are not twenty "Configure"s', () => {
    renderRow(false);
    const button = screen.getByRole('button', {
      name: catalogCopy.configureRow('Versa API Azure'),
    });
    expect(button).toHaveTextContent(catalogCopy.configure);
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
