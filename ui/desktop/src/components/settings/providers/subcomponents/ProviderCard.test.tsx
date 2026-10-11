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

  it('swaps "Configured" for the action in one cell, so the right edge never moves', () => {
    renderRow(true);
    const status = screen.getByText(catalogCopy.configured);
    const slot = status.closest('.br-row-actions-slot');
    expect(slot, 'the status and the action share the shared reveal cell').not.toBeNull();
    // The status is the cell's `meta` (shown at rest, gives way on hover and focus); the button is
    // the cluster that takes its place.
    expect(status).toHaveClass('br-row-actions-meta');
    expect(slot).toContainElement(screen.getByTestId('provider-actions-versa_azure'));
  });

  // V4-29: first run reveals a row's action the way Settings does (principle 6), so the two
  // surfaces are one list. The row acts on a click there too, so a button at opacity 0 is never
  // the only way in.
  it('reveals the action the same way during first run, and the row itself acts', async () => {
    const user = userEvent.setup();
    const onConfigure = vi.fn();
    const onLaunch = vi.fn();
    const { rerender } = render(
      <TooltipProvider>
        <ProviderCard
          provider={provider(true)}
          onConfigure={onConfigure}
          onLaunch={onLaunch}
          isOnboarding
        />
      </TooltipProvider>
    );
    expect(screen.getByTestId('provider-actions-versa_azure')).toHaveClass('br-row-actions');
    expect(screen.getByText(catalogCopy.configured)).toHaveClass('br-row-actions-meta');
    expect(screen.getByText(catalogCopy.configured).closest('.br-row-actions-slot')).not.toBeNull();

    // A configured provider is used, as its button says: "Use", the verb the open panels use
    // ("Use Claude Code"), named for the row (vision FR-R2-4: it read "Launch").
    const use = screen.getByRole('button', { name: catalogCopy.useRow('Versa API Azure') });
    expect(use).toHaveTextContent(/^Use$/);
    expect(screen.queryByText('Launch')).toBeNull();
    await user.click(screen.getByTestId('provider-card-versa_azure'));
    expect(onLaunch).toHaveBeenCalledTimes(1);
    expect(onConfigure).not.toHaveBeenCalled();
    await user.click(use);
    expect(onLaunch).toHaveBeenCalledTimes(2);

    // One that is not set up is configured.
    rerender(
      <TooltipProvider>
        <ProviderCard
          provider={provider(false)}
          onConfigure={onConfigure}
          onLaunch={onLaunch}
          isOnboarding
        />
      </TooltipProvider>
    );
    await user.click(screen.getByTestId('provider-card-versa_azure'));
    expect(onConfigure).toHaveBeenCalledTimes(1);
  });

  // FR-R1-5: on first run an expandable row's panel holds the primary ("Use Claude Code",
  // "Download and start", "Connect"); a hover row action beside it was a second verb for one choice.
  it('gives an expandable first-run row no row action, and keeps "Configured" at rest', () => {
    render(
      <TooltipProvider>
        <ProviderCard
          provider={provider(true)}
          onConfigure={vi.fn()}
          onLaunch={vi.fn()}
          isOnboarding
          expandable
          expanded
          onToggle={vi.fn()}
        >
          <button type="button">Use it</button>
        </ProviderCard>
      </TooltipProvider>
    );
    expect(screen.queryByTestId('provider-actions-versa_azure')).toBeNull();
    expect(
      screen.queryByRole('button', { name: catalogCopy.useRow('Versa API Azure') })
    ).toBeNull();
    const status = screen.getByText(catalogCopy.configured);
    // Nothing takes its cell, so it does not give way on hover.
    expect(status).not.toHaveClass('br-row-actions-meta');
    expect(screen.getByRole('button', { name: 'Use it' })).toBeInTheDocument();
  });

  it('keeps the action on an expandable row in Settings', () => {
    render(
      <TooltipProvider>
        <ProviderCard
          provider={provider(true)}
          onConfigure={vi.fn()}
          onLaunch={vi.fn()}
          isOnboarding={false}
          expandable
          onToggle={vi.fn()}
        />
      </TooltipProvider>
    );
    expect(screen.getByTestId('provider-actions-versa_azure')).toBeInTheDocument();
    expect(screen.getByText(catalogCopy.configured)).toHaveClass('br-row-actions-meta');
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

  /**
   * jsdom lays nothing out, so every width reads 0 and nothing is ever "truncated". These cases
   * stand the description in for a cut-off one: wider text than box.
   */
  const truncateDescriptions = () => {
    const isDescription = (el: HTMLElement) =>
      (el.dataset?.testid ?? '').startsWith('provider-description-');
    const scroll = vi
      .spyOn(HTMLElement.prototype, 'scrollWidth', 'get')
      .mockImplementation(function (this: HTMLElement) {
        return isDescription(this) ? 400 : 0;
      });
    const client = vi
      .spyOn(HTMLElement.prototype, 'clientWidth', 'get')
      .mockImplementation(function (this: HTMLElement) {
        return isDescription(this) ? 200 : 0;
      });
    return () => {
      scroll.mockRestore();
      client.mockRestore();
    };
  };

  // FR-R1-9: every description was a Tab stop, cut off or not, so the walk was row, description,
  // action for every provider. Only a cut-off one is a stop now; the action carries the full text.
  it('is not a Tab stop when the description fits, and the action is described by it', async () => {
    const user = userEvent.setup();
    renderRow(true);
    expect(screen.getByTestId('provider-description-versa_azure')).not.toHaveAttribute('tabindex');
    await user.tab();
    const configure = screen.getByRole('button');
    expect(configure).toHaveFocus();
    expect(configure).toHaveAccessibleDescription(
      'A description long enough that the row truncates it in a narrow window'
    );
  });

  it('reaches a cut-off description and then Configure by Tab, and Enter opens the dialog', async () => {
    const restore = truncateDescriptions();
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
    restore();
  });

  it('opens a cut-off description in a tooltip on keyboard focus', async () => {
    const restore = truncateDescriptions();
    const user = userEvent.setup();
    renderRow(false);
    await user.tab();
    expect(screen.getByTestId('provider-description-versa_azure')).toHaveFocus();
    expect(
      await screen.findByRole('tooltip', {
        name: /A description long enough that the row truncates it/,
      })
    ).toBeInTheDocument();
    restore();
  });

  it('describes an agent row by the daemon sentence its short line stands in for', () => {
    render(
      <TooltipProvider>
        <ProviderCard
          provider={provider(false)}
          onConfigure={vi.fn()}
          onLaunch={vi.fn()}
          isOnboarding
          expandable
          onToggle={vi.fn()}
          summary="Your plan · no API key"
        />
      </TooltipProvider>
    );
    const toggle = screen.getByTestId('provider-row-toggle-versa_azure');
    expect(toggle).toHaveTextContent('Your plan · no API key');
    expect(toggle).toHaveAccessibleDescription(
      'A description long enough that the row truncates it in a narrow window'
    );
  });
});
