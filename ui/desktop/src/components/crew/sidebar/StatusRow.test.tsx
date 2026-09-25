import { fireEvent, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { forgetJoinContext, updateJoinContext } from '../onboarding/joinContext';
import { crewStatusCopy } from '../state/copy';
import { CONNECTION_STATUS, type ConnectionStatusKey } from '../state/crewStatus';
import { sidebarCopy } from './copy';
import { statusHint, StatusRow } from './StatusRow';
import { connection, makeController, renderWithCrew } from './sidebarTestUtils';

const config = vi.hoisted(() => ({
  getProviders: async () => [],
  read: async () => null,
}));
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return { ...actual, useConfig: () => config };
});

afterEach(() => {
  forgetJoinContext(connection.id);
});

const unverified = { snapshot: null, observedPrivacy: null, effectivePrivacy: null } as const;

const STATUS_KEYS = Object.keys(CONNECTION_STATUS) as ConnectionStatusKey[];

function statusRegion() {
  return screen.getByRole('status', { name: sidebarCopy.statusLabel });
}

describe('StatusRow', () => {
  it.each(STATUS_KEYS)('shows the word for %s beside its dot or spinner', (status) => {
    const presentation = CONNECTION_STATUS[status];
    const { container } = renderWithCrew(<StatusRow />, makeController({ status }));
    const region = statusRegion();

    expect(region).toHaveTextContent(presentation.word);
    expect(container.querySelector('[data-crew-status]')).toHaveAttribute(
      'data-crew-status',
      status
    );
    const dot = region.querySelector('[data-slot="status-dot"]');
    const spinner = region.querySelector('.crew-sidebar-spinner');
    if (presentation.spinner) {
      expect(spinner).not.toBeNull();
      expect(dot).toBeNull();
    } else {
      expect(spinner).toBeNull();
      // The dot sits beside a word, so it is hidden from assistive technology.
      expect(dot).toHaveAttribute('data-tone', presentation.tone);
      expect(dot).toHaveAttribute('aria-hidden', 'true');
    }
  });

  it('reads the pinned verified sentence to a screen reader when connected, once', () => {
    renderWithCrew(<StatusRow />, makeController({ status: 'connected' }));
    // The regression tests find this exact standalone text node.
    const verified = screen.getByText(crewStatusCopy.verified);
    expect(verified).toHaveClass('sr-only');
    expect(screen.getAllByText(crewStatusCopy.verified)).toHaveLength(1);
    // The short visible word is hidden from assistive technology so it is not said twice.
    expect(screen.getByText(crewStatusCopy.connected)).toHaveAttribute('aria-hidden', 'true');
  });

  it('shows the pinned checking and updates-unavailable words as standalone text nodes', () => {
    const view = renderWithCrew(<StatusRow />, makeController({ status: 'checking' }));
    expect(screen.getByText(crewStatusCopy.checking)).toBeInTheDocument();
    view.update(makeController({ status: 'updates-unavailable' }));
    expect(screen.getByText(crewStatusCopy.updatesUnavailable)).toBeInTheDocument();
    expect(screen.queryByText(crewStatusCopy.checking)).toBeNull();
  });

  it.each(
    STATUS_KEYS.filter(
      (key) => key !== 'sign-in-needed' && key !== 'updates-unavailable' && key !== 'not-joined'
    )
  )('never gives %s a tooltip that repeats its own word (Q2-17)', (status) => {
    renderWithCrew(<StatusRow />, makeController({ status }));
    const word = document.querySelector('[data-crew-status-word]') as HTMLElement;
    expect(word).toHaveTextContent(CONNECTION_STATUS[status].word);
    // No native title either: the app's tooltip layer would place it over the switcher above.
    expect(word).not.toHaveAttribute('title');
    expect(statusHint(status, 'Fixture', null)).toBeNull();
  });

  it('says where the fix is for "Updates unavailable", below the row and to a screen reader', async () => {
    const user = userEvent.setup();
    renderWithCrew(<StatusRow />, makeController({ ...unverified, status: 'updates-unavailable' }));
    const hint = 'Crew isn’t receiving updates for Fixture. Retry below.';
    expect(sidebarCopy.statusHint.updatesUnavailable('Fixture')).toBe(hint);
    // The status region carries it after the word, so it is heard with the status.
    expect(statusRegion()).toHaveTextContent(`${crewStatusCopy.updatesUnavailable}. ${hint}`);
    const word = document.querySelector('[data-crew-status-word]') as HTMLElement;
    expect(word).not.toHaveAttribute('title');
    await user.hover(word);
    const tooltip = await screen.findByRole('tooltip');
    expect(tooltip).toHaveTextContent(hint);
    // Below the row, never up over the workspace name.
    const content = document.querySelector('[data-crew-status-tooltip]') as HTMLElement;
    expect(content).toHaveAttribute('data-side', 'bottom');
  });

  it('says whose turn it is while a join waits, and nothing about privacy (Q2-43)', async () => {
    const user = userEvent.setup();
    updateJoinContext(connection.id, { hostUsername: 'alice', hostDisplayName: null });
    const { container } = renderWithCrew(
      <StatusRow />,
      makeController({ ...unverified, status: 'not-joined' })
    );
    const region = statusRegion();
    expect(within(region).getByText(crewStatusCopy.notJoined)).toBeInTheDocument();
    expect(region.textContent).toMatch(/Waiting for .*@alice.* to let you in$/);
    // Only the status: no "Privacy shown after…" truncated beside it.
    expect(container).not.toHaveTextContent(sidebarCopy.chip.notJoined);
    expect(container).not.toHaveTextContent(sidebarCopy.chip.checking);
    expect(container.querySelector('[data-crew-privacy]')).toBeNull();
    await user.hover(within(region).getByText(crewStatusCopy.notJoined));
    expect((await screen.findByRole('tooltip')).textContent).toMatch(
      /^Waiting for .*@alice.* to let you in$/
    );
  });

  it('names no one it cannot: "your host" when the join remembered no host', () => {
    renderWithCrew(<StatusRow />, makeController({ ...unverified, status: 'not-joined' }));
    expect(statusRegion()).toHaveTextContent('Waiting for your host to let you in');
  });

  it.each(['offline', 'reconnecting', 'sign-in-needed', 'cant-connect'] as const)(
    'shows no privacy text beside %s, which contradicted it (Q2-17)',
    (status) => {
      const { container } = renderWithCrew(
        <StatusRow />,
        makeController({ ...unverified, status })
      );
      expect(container).not.toHaveTextContent(sidebarCopy.chip.checking);
      expect(container.querySelector('[data-crew-privacy]')).toBeNull();
    }
  );

  // Q3-55: at 240px the row read "Checking connection · Checking pri…" and "Connecting… ·
  // Checking privacy…". The word says a check runs, so the unverified chip steps aside, and the
  // status region still says both facts.
  it.each([
    ['checking', crewStatusCopy.checking],
    ['connecting', crewStatusCopy.connecting],
  ] as const)(
    'reads "%s" whole, and tells a screen reader privacy is being checked too',
    (status, word) => {
      const { container } = renderWithCrew(
        <StatusRow />,
        makeController({ ...unverified, status })
      );
      expect(container.querySelector('[data-crew-privacy]')).toBeNull();
      const visible = document.querySelector('[data-crew-status-word]') as HTMLElement;
      expect(visible).toHaveTextContent(word);
      // The only text beside the word is the screen reader's, and it keeps both facts.
      const more = statusRegion().querySelector('[data-crew-status-more]') as HTMLElement;
      expect(more).toHaveClass('sr-only');
      expect(statusRegion()).toHaveTextContent(`${word}. ${sidebarCopy.chip.checking}`);
    }
  );

  it('says nothing extra once privacy is verified, even during a connect', () => {
    renderWithCrew(<StatusRow />, makeController({ status: 'connecting' }));
    expect(statusRegion()).not.toHaveTextContent(sidebarCopy.chip.checking);
    expect(screen.getByRole('button', { name: 'Privacy: Private · ucsf' })).toBeInTheDocument();
  });

  it('makes "Sign-in needed" a button that opens Sign in', () => {
    const controller = makeController({ status: 'sign-in-needed' });
    renderWithCrew(<StatusRow />, controller);
    const button = within(statusRegion()).getByRole('button', {
      name: crewStatusCopy.signInNeeded,
    });
    expect(button).toHaveClass('no-drag');
    fireEvent.click(button);
    expect(controller.openSignIn).toHaveBeenCalledTimes(1);
  });

  it('offers no button for any other status', () => {
    for (const status of STATUS_KEYS.filter((key) => key !== 'sign-in-needed')) {
      const view = renderWithCrew(<StatusRow />, makeController({ status }));
      expect(within(statusRegion()).queryByRole('button')).toBeNull();
      view.unmount();
    }
  });

  it('renders nothing until a connection is selected', () => {
    const { container } = renderWithCrew(
      <StatusRow />,
      makeController({ connection: null, status: null })
    );
    expect(container).toBeEmptyDOMElement();
  });

  it('puts the privacy chip on the same row', () => {
    renderWithCrew(<StatusRow />);
    expect(screen.getByRole('button', { name: 'Privacy: Private · ucsf' })).toBeInTheDocument();
  });
});
