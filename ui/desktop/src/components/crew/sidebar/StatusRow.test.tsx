import { fireEvent, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { crewStatusCopy } from '../state/copy';
import { CONNECTION_STATUS, type ConnectionStatusKey } from '../state/crewStatus';
import { sidebarCopy } from './copy';
import { StatusRow } from './StatusRow';
import { makeController, renderWithCrew } from './sidebarTestUtils';

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

  it.each(STATUS_KEYS.filter((key) => key !== 'sign-in-needed'))(
    'carries the full words of %s in a tooltip, since the row can truncate them (T-68)',
    (status) => {
      const presentation = CONNECTION_STATUS[status];
      renderWithCrew(<StatusRow />, makeController({ status }));
      const word = within(statusRegion()).getByText(presentation.word);
      expect(word).toHaveAttribute('title', presentation.srText ?? presentation.word);
    }
  );

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
