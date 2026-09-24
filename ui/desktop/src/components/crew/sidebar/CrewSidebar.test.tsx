import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { Invitation } from '../crewApi';
import { crewStatusCopy } from '../state/copy';
import { CrewSidebar } from './CrewSidebar';
import { sidebarCopy } from './copy';
import {
  alice,
  bob,
  connection,
  makeController,
  makeSnapshot,
  renderWithCrew,
  TEAM_LAB,
} from './sidebarTestUtils';

const invitation: Invitation = {
  id: 'invitation-1',
  kind: 'team',
  target_id: 'team-imaging-0000',
  principal_id: alice.id,
  inviter_id: bob.id,
  expires_at: 4_000_000_000,
  target_name: 'Imaging Core',
};

describe('CrewSidebar', () => {
  it('is the Crew navigation landmark, top to bottom: switcher, status, sections, slot, You', () => {
    renderWithCrew(
      <CrewSidebar agentsSection={<div data-testid="agents-slot">Agents</div>} />,
      makeController({ snapshot: makeSnapshot({ invitations: [invitation] }) })
    );
    const nav = screen.getByRole('navigation', { name: sidebarCopy.navLabel });
    const order = [
      within(nav).getByRole('button', { name: /^Fixture/ }),
      within(nav).getByRole('status', { name: sidebarCopy.statusLabel }),
      within(nav).getByRole('heading', { name: sidebarCopy.section.invitations }),
      within(nav).getByRole('button', { name: /^Analysis Lab, / }),
      within(nav).getByTestId('agents-slot'),
      within(nav).getByText('alice@hpc.ucsf.edu'),
    ];
    for (let index = 1; index < order.length; index += 1) {
      expect(
        order[index - 1].compareDocumentPosition(order[index]) & Node.DOCUMENT_POSITION_FOLLOWING
      ).toBeTruthy();
    }
    // The places and the footer sit outside the scrolling middle, which holds the sections.
    const scroll = nav.querySelector('[data-crew-sidebar-scroll]') as HTMLElement;
    expect(scroll).toContainElement(within(nav).getByTestId('agents-slot'));
    expect(scroll).not.toContainElement(within(nav).getByText('alice@hpc.ucsf.edu'));
  });

  it('shows the pinned verified sentence exactly once at rest', () => {
    renderWithCrew(<CrewSidebar />);
    expect(screen.getAllByText(crewStatusCopy.verified)).toHaveLength(1);
    expect(screen.getAllByText('alice@hpc.ucsf.edu')).toHaveLength(1);
  });

  it('keeps the places from the last verified copy during a refresh, never its security state', () => {
    const snapshot = makeSnapshot();
    renderWithCrew(
      <CrewSidebar />,
      makeController({
        snapshot: null,
        observedPrivacy: null,
        effectivePrivacy: null,
        status: 'checking',
        lastVerified: {
          connectionId: connection.id,
          snapshot,
          observedPrivacy: {
            connectionId: connection.id,
            mode: 'private',
            institutionId: 'ucsf',
            policyEpoch: 1,
          },
          runs: [],
          labels: null,
          teamId: TEAM_LAB,
          channelId: 'chan-methods',
          messages: [],
        },
      })
    );
    expect(screen.getByRole('button', { name: 'methods' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByText(crewStatusCopy.checking)).toBeInTheDocument();
    expect(screen.getByText(sidebarCopy.chip.checking)).toBeInTheDocument();
    expect(screen.queryByTestId('privacy-badge')).toBeNull();
    expect(screen.getByRole('button', { name: sidebarCopy.team.add })).toBeDisabled();
  });

  it('shows only the switcher band until a connection is selected', () => {
    renderWithCrew(<CrewSidebar />, makeController({ connection: null, status: null }));
    const nav = screen.getByRole('navigation', { name: sidebarCopy.navLabel });
    expect(nav).toHaveAttribute('aria-busy', 'true');
    expect(within(nav).getByRole('button', { name: sidebarCopy.switcher.loading })).toBeDisabled();
    expect(within(nav).queryByRole('status')).toBeNull();
  });

  it('declares no drag region, and every band control opts out of one', () => {
    const { container } = renderWithCrew(<CrewSidebar />);
    for (const element of Array.from(container.querySelectorAll<HTMLElement>('*'))) {
      expect(element.getAttribute('style') ?? '').not.toMatch(/app-region\s*:\s*drag/);
    }
    const band = container.querySelector('[data-crew-band="switcher"]') as HTMLElement;
    for (const control of Array.from(band.querySelectorAll('button'))) {
      expect(control).toHaveClass('no-drag');
    }
    const status = container.querySelector('.crew-sidebar-status') as HTMLElement;
    for (const control of Array.from(status.querySelectorAll('button'))) {
      expect(control).toHaveClass('no-drag');
    }
  });
});

/**
 * jsdom evaluates none of `crew-sidebar.css`, so the two properties a real window depends on are
 * held at the source: the titlebar reserve is a MARGIN keyed on the app sidebar's collapsed state
 * (issue #74 — padding would stay inside the box the controls sit over), and nothing in the
 * sidebar's stylesheet declares a drag region.
 */
describe('crew-sidebar.css', () => {
  const css = readFileSync(join(__dirname, 'crew-sidebar.css'), 'utf8').replace(
    /\/\*[\s\S]*?\*\//g,
    ' '
  );

  it('reserves the titlebar controls with a margin when the app sidebar is collapsed', () => {
    const rule = /([^{}]*\.crew-sidebar-switcher)\s*\{([^}]*)\}/g;
    const reserves = Array.from(css.matchAll(rule)).filter(([, , body]) =>
      body.includes('--biorouter-titlebar-control-reserve')
    );
    expect(reserves).toHaveLength(1);
    const [, selector, body] = reserves[0];
    expect(selector.replace(/\s+/g, ' ').trim()).toBe(
      "[data-slot='sidebar'][data-state='collapsed'] ~ [data-slot='sidebar-inset'] .crew-sidebar-switcher"
    );
    expect(body).toMatch(/margin-left\s*:\s*var\(--biorouter-titlebar-control-reserve\)/);
    expect(body).not.toMatch(/padding/);
  });

  it('declares no -webkit-app-region at all', () => {
    expect(css).not.toMatch(/app-region/);
  });
});
