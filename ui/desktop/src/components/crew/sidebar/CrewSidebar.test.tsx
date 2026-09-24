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

  it('explains the arrow keys on the landmark, since Tab reaches only one row (T-64)', () => {
    renderWithCrew(<CrewSidebar />);
    const nav = screen.getByRole('navigation', { name: sidebarCopy.navLabel });
    expect(nav).toHaveAttribute('aria-description', sidebarCopy.navDescription);
    expect(sidebarCopy.navDescription).toMatch(/Up and Down arrow keys/);
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
  const stripComments = (source: string) => source.replace(/\/\*[\s\S]*?\*\//g, ' ');
  const css = stripComments(readFileSync(join(__dirname, 'crew-sidebar.css'), 'utf8'));
  const appCss = stripComments(readFileSync(join(__dirname, '..', 'crew-app.css'), 'utf8'));

  /** The declarations of every rule whose selector list is exactly `selector`. */
  function bodiesOf(source: string, selector: string): string[] {
    const normalize = (text: string) => text.replace(/\s+/g, ' ').trim();
    return Array.from(source.matchAll(/([^{}]+)\{([^{}]*)\}/g))
      .filter(([, found]) => normalize(found) === normalize(selector))
      .map(([, , body]) => body);
  }

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

  /**
   * T-21: inside a fixed 240px column the 172px macOS reserve left the switcher 51px ("c.").
   * The column grows by the reserve under the SAME collapsed state, and the name keeps 10ch.
   */
  it('widens the Crew column by the titlebar reserve while the app sidebar is collapsed', () => {
    const [body] = bodiesOf(
      appCss,
      "[data-slot='sidebar'][data-state='collapsed'] ~ [data-slot='sidebar-inset'] .crew-app"
    );
    expect(body).toBeDefined();
    expect(body).toMatch(
      /--crew-sidebar-width\s*:\s*calc\(240px \+ var\(--biorouter-titlebar-control-reserve\) - 16px\)/
    );
    // The column is sized by that property, and 240px stays the resting width.
    expect(bodiesOf(appCss, '.crew-app')[0]).toMatch(/--crew-sidebar-width:\s*240px/);
    expect(bodiesOf(appCss, '.crew-app')[0]).toMatch(
      /grid-template-columns:\s*var\(--crew-sidebar-width\)/
    );
    expect(bodiesOf(css, '.crew-sidebar-switcher-name')[0]).toMatch(/min-width:\s*10ch/);
  });

  /**
   * T-16: the focus fill alone is 1.10–1.44:1 against what it sits on. Every focusable row
   * also draws the inset accent edge, and keeps the fill for hover.
   */
  it.each([
    '.crew-sidebar-row',
    '.crew-sidebar-switcher',
    '.crew-sidebar-team-toggle',
    '.crew-sidebar-you-trigger',
  ])('gives %s:focus-visible the inset accent edge', (selector) => {
    const [focus] = bodiesOf(css, `${selector}:focus-visible`);
    expect(focus).toMatch(/box-shadow:\s*inset 0 0 0 2px var\(--border-accent\)/);
    expect(focus).toMatch(/outline:\s*none/);
    const [hover] = bodiesOf(css, `${selector}:hover`);
    if (selector !== '.crew-sidebar-team-toggle') {
      expect(hover).toMatch(/background-color/);
      expect(hover).not.toMatch(/box-shadow/);
    }
  });

  it('gives a focused text field in Crew the accent edge, never over a danger edge', () => {
    const [body] = bodiesOf(
      appCss,
      ".crew-app :read-write:focus-visible:not([aria-invalid='true'])"
    );
    expect(body).toMatch(/border-color:\s*var\(--border-accent\)/);
  });

  it('keeps the selected channel’s bar in forced colours (T-58)', () => {
    const block = /@media \(forced-colors: active\)\s*\{([\s\S]*?)\}\s*\}/.exec(css)?.[1] ?? '';
    expect(block).toMatch(/\.crew-sidebar-row\[aria-current='page'\]::before\s*\{/);
    expect(block).toMatch(/background-color:\s*Highlight/);
    expect(block).toMatch(/forced-color-adjust:\s*none/);
  });

  it('matches the app sidebar’s rhythm and tints the whole team header (T-62)', () => {
    const [row] = bodiesOf(css, '.crew-sidebar-row');
    expect(row).toMatch(/font-size:\s*var\(--text-body\)/);
    expect(bodiesOf(css, ".crew-sidebar-row[aria-current='page']")[0]).toMatch(
      /font-weight:\s*500/
    );
    const [list] = bodiesOf(css, '.crew-sidebar-list');
    expect(list).toMatch(/padding:\s*0 8px/);
    expect(list).toMatch(/gap:\s*2px/);
    expect(bodiesOf(css, '.crew-sidebar-team-header:hover')[0]).toMatch(/background-color/);
    expect(bodiesOf(css, '.crew-sidebar-team-toggle:hover')).toEqual([]);
    expect(bodiesOf(css, '.crew-sidebar-you-trigger')[0]).toMatch(
      /transition:\s*background-color var\(--dur-fast-min\) var\(--ease-out\)/
    );
  });
});
