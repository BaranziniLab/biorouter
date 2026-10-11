import { createPortal } from 'react-dom';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ProviderSettings from './ProviderSettingsPage';
import { providerPageCopy } from './copy';
import { KEY_STORAGE_NOTICE } from './modal/subcomponents/SecureStorageNotice';

/**
 * The provider page's band (codex simplicity redesign, spec 3.13; vision round 1, V4-14 and
 * V4-29): the shared 44px band, Back as its icon action, a 14px title whose InfoTip says where keys
 * are kept (and, on first run, that the choice is not final), and the catalog's three tabs drawn
 * into the band's tab slot. No paragraph under the title. The catalog it hosts has its own suites.
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

// The catalog draws its tab list into the band's slot (`tabsHost`); the stand-in does the same
// with a marker, which is what proves the slot is the band's.
vi.mock('./ProviderCatalog', () => ({
  default: ({ mode, tabsHost }: { mode: string; tabsHost?: HTMLElement | null }) => (
    <div data-testid="catalog">
      {mode}
      {tabsHost ? createPortal(<span data-testid="tabs-marker" />, tabsHost) : null}
    </div>
  ),
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

  it('outside first run is a band with the title and where keys are kept behind its InfoTip', async () => {
    renderPage(false);
    expect(await screen.findByTestId('catalog')).toHaveTextContent('settings');

    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(providerPageCopy.title);
    const tip = screen.getByRole('button', { name: `About ${providerPageCopy.title}` });
    expect(tip).toBeInTheDocument();
    // The key notice is reachable from the page without a hover (the tip's hidden description).
    expect(tip).toHaveAccessibleDescription(KEY_STORAGE_NOTICE);
    expect(screen.getByText(KEY_STORAGE_NOTICE)).toBeInTheDocument();
    // No description paragraph under the title.
    expect(screen.queryByText(/Configure your AI model providers/)).toBeNull();
  });

  it('during first run says nothing under the title: the lead is the InfoTip’s', async () => {
    renderPage(true);
    expect(await screen.findByTestId('catalog')).toHaveTextContent('onboarding');

    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(
      providerPageCopy.onboardingTitle
    );
    // V4-29: a visible line that restated the title is gone. What it said, that the choice is not
    // final, sits ahead of where keys are kept in the same tip, and is still read with the control.
    const tip = screen.getByRole('button', { name: `About ${providerPageCopy.onboardingTitle}` });
    expect(tip).toHaveAccessibleDescription(
      `${providerPageCopy.onboardingHelp} ${KEY_STORAGE_NOTICE}`
    );
    expect(screen.queryByText('Pick where your models run.', { exact: false })).toBeNull();
    const band = screen.getByTestId('page-header');
    expect(band.parentElement?.querySelectorAll('p')).toHaveLength(0);
  });

  it('is one band: Back is its icon action and the catalog’s tabs sit in its tab slot', async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(
      <MemoryRouter>
        <ProviderSettings onClose={onClose} isOnboarding={false} />
      </MemoryRouter>
    );
    const band = screen.getByTestId('page-header');
    // The tab list is drawn into the band, not beside the rows.
    expect(await screen.findByTestId('tabs-marker')).toBeInTheDocument();
    expect(band).toContainElement(screen.getByTestId('tabs-marker'));
    expect(band.querySelector('.br-provider-band-tabs-host')).not.toBeNull();

    await user.click(screen.getByRole('button', { name: 'Back' }));
    expect(onClose).toHaveBeenCalledTimes(1);
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
