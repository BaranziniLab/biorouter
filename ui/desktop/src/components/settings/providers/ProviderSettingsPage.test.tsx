import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ProviderSettings from './ProviderSettingsPage';
import { providerPageCopy } from './copy';
import { KEY_STORAGE_NOTICE } from './modal/subcomponents/SecureStorageNotice';

/**
 * The provider page's header (codex simplicity redesign, spec 3.13): a back button and a 24px
 * title, where keys are kept as the title's InfoTip, and, only during first run, the one line
 * that says what to do. The catalog it hosts has its own suites.
 */

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  read: vi.fn(),
  upsert: vi.fn(),
}));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    getProviders: mocks.getProviders,
    read: mocks.read,
    upsert: mocks.upsert,
  }),
}));

vi.mock('./ProviderCatalog', () => ({
  default: ({ mode }: { mode: string }) => <div data-testid="catalog">{mode}</div>,
}));

vi.mock('../../onboarding/CommercialSetupCard', () => ({
  persistDetectedProviderSetup: vi.fn(),
}));

function renderPage(isOnboarding: boolean) {
  return render(
    <MemoryRouter>
      <ProviderSettings onClose={vi.fn()} isOnboarding={isOnboarding} />
    </MemoryRouter>
  );
}

describe('the provider page', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getProviders.mockResolvedValue([]);
    mocks.read.mockResolvedValue('');
  });

  it('outside first run is a title with where keys are kept behind its InfoTip', async () => {
    renderPage(false);
    expect(await screen.findByTestId('catalog')).toHaveTextContent('settings');

    expect(screen.getByTestId('provider-selection-heading')).toHaveTextContent(
      providerPageCopy.title
    );
    const tip = screen.getByRole('button', { name: `About ${providerPageCopy.title}` });
    expect(tip).toBeInTheDocument();
    // The key notice is reachable from the page without a hover (the tip's hidden description).
    expect(screen.getByText(KEY_STORAGE_NOTICE)).toBeInTheDocument();
    // No description paragraph under the title.
    expect(screen.queryByText(providerPageCopy.onboardingLead)).toBeNull();
    expect(screen.queryByText(/Configure your AI model providers/)).toBeNull();
  });

  it('during first run keeps one lead-in line and the same InfoTip', async () => {
    renderPage(true);
    expect(await screen.findByTestId('catalog')).toHaveTextContent('onboarding');

    expect(screen.getByTestId('provider-selection-heading')).toHaveTextContent(
      providerPageCopy.onboardingTitle
    );
    expect(screen.getByText(providerPageCopy.onboardingLead)).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: `About ${providerPageCopy.onboardingTitle}` })
    ).toBeInTheDocument();
  });

  it('stands on the same ground as every other view', async () => {
    const { container } = renderPage(false);
    await screen.findByTestId('catalog');
    const root = container.firstElementChild as HTMLElement;
    expect(root.className).toContain('bg-background-canvas');
    expect(root.className).not.toContain('bg-background-muted');
  });

  it('shows skeleton rows, not a line of text, while the providers load', () => {
    mocks.getProviders.mockReturnValue(new Promise(() => {}));
    renderPage(false);
    expect(screen.getByRole('status', { name: providerPageCopy.loading })).toBeInTheDocument();
    expect(screen.queryByTestId('catalog')).toBeNull();
  });
});
