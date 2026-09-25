import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Invitation } from '../crewApi';
import { forgetJoinContext, updateJoinContext } from '../onboarding/joinContext';
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
    // …and how a team's own buttons are reached, which arrows never do (Q2-46).
    expect(sidebarCopy.navDescription).toMatch(
      /On a team, Tab reaches its Create channel and options buttons\.$/
    );
  });

  it('tells a joiner what the empty column will hold, and who it waits for (Q2-43)', () => {
    updateJoinContext(connection.id, { hostUsername: 'alice', hostDisplayName: null });
    renderWithCrew(
      <CrewSidebar />,
      makeController({
        snapshot: null,
        observedPrivacy: null,
        effectivePrivacy: null,
        status: 'not-joined',
      })
    );
    const scroll = document.querySelector('[data-crew-sidebar-scroll]') as HTMLElement;
    const note = scroll.querySelector('[data-crew-sidebar-pending]') as HTMLElement;
    expect(note.textContent).toMatch(/^Your channels appear here once .*@alice.* lets you in\.$/);
    // Not a row, and nothing to press.
    expect(within(scroll).queryByRole('button')).toBeNull();
  });

  it('says nothing of the kind to a member', () => {
    renderWithCrew(<CrewSidebar />);
    expect(document.querySelector('[data-crew-sidebar-pending]')).toBeNull();
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
 * jsdom evaluates none of `crew-sidebar.css`, so the properties a real window depends on are held
 * at the source (and measured in Chromium by `crewSidebarGeometry.browser.test.ts`): while the
 * app sidebar is collapsed the switcher moves to a row of its own below the titlebar band (issue
 * #74, Q2-39) and the column stays 240px, and nothing in the sidebar's stylesheet declares a drag
 * region.
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

  const COLLAPSED = "[data-slot='sidebar'][data-state='collapsed'] ~ [data-slot='sidebar-inset']";

  /**
   * Q2-39: the switcher leaves the band for its own 36px row while the app sidebar is collapsed,
   * so nothing of Crew's sits under the titlebar controls — and it is never pushed right by a
   * margin, which in a 240px column left it 51px ("c.").
   */
  it('moves the switcher below the band while the app sidebar is collapsed, never beside it', () => {
    const [band] = bodiesOf(css, `${COLLAPSED} .crew-sidebar-band`);
    expect(band).toBeDefined();
    expect(band).toMatch(/grid-template-rows:\s*var\(--chrome-height\) 36px/);
    expect(band).toMatch(/height:\s*calc\(var\(--chrome-height\) \+ 36px\)/);
    // The band's hairline stays at y=44, the channel header's: one continuous top edge.
    expect(band).toMatch(/background-position:\s*0 calc\(var\(--chrome-height\) - 1px\)/);
    expect(bodiesOf(css, `${COLLAPSED} .crew-sidebar-switcher`)[0]).toMatch(/grid-row:\s*2/);
    // At rest the band is the 44px band, and the switcher sits in it.
    expect(bodiesOf(css, '.crew-sidebar-band')[0]).toMatch(/height:\s*var\(--chrome-height\)/);
    // No margin reserve anywhere: the switcher keeps the band's whole width.
    expect(css).not.toMatch(/margin-left\s*:\s*var\(--biorouter-titlebar-control-reserve\)/);
  });

  it('declares no -webkit-app-region at all', () => {
    expect(css).not.toMatch(/app-region/);
  });

  /**
   * Q2-39: round 1 widened the column by the reserve (T-21) to 396px, which left the channel 652px
   * at a 1048px window, so the details pane covered it. The column is 240px in every state.
   */
  it('keeps the Crew column 240px whether or not the app sidebar is collapsed', () => {
    expect(bodiesOf(appCss, `${COLLAPSED} .crew-app`)).toEqual([]);
    expect(appCss).not.toMatch(/--crew-sidebar-width\s*:\s*calc\(/);
    expect(bodiesOf(appCss, '.crew-app')[0]).toMatch(/--crew-sidebar-width:\s*240px/);
    expect(bodiesOf(appCss, '.crew-app')[0]).toMatch(
      /grid-template-columns:\s*var\(--crew-sidebar-width\)/
    );
    expect(bodiesOf(css, '.crew-sidebar-switcher-name')[0]).toMatch(/min-width:\s*10ch/);
  });

  /**
   * T-16: the focus fill alone is 1.10–1.44:1 against what it sits on. Every focusable row
   * also draws the app's neutral inset focus edge (`--border-focus`, ≥4.15:1 on every
   * sidebar ground in all six scopes, asserted by `check-contrast.mjs`), and keeps the fill
   * for hover. `--border-accent` is what this used to draw, and Roche Limit light's measures
   * 2.33:1 on the focus fill and 2.88:1 on `--sidebar` — under SC 1.4.11's 3:1.
   */
  it.each([
    '.crew-sidebar-row',
    '.crew-sidebar-switcher',
    '.crew-sidebar-team-toggle',
    '.crew-sidebar-you-trigger',
  ])('gives %s:focus-visible the inset focus edge', (selector) => {
    const [focus] = bodiesOf(css, `${selector}:focus-visible`);
    expect(focus).toMatch(/box-shadow:\s*inset 0 0 0 2px var\(--border-focus\)/);
    expect(focus).toMatch(/outline:\s*none/);
    const [hover] = bodiesOf(css, `${selector}:hover`);
    if (selector !== '.crew-sidebar-team-toggle') {
      expect(hover).toMatch(/background-color/);
      expect(hover).not.toMatch(/box-shadow/);
    }
  });

  it('never draws a sidebar focus state in the accent edge', () => {
    const uncommented = css.replace(/\/\*[\s\S]*?\*\//g, '');
    const focusRules = Array.from(uncommented.matchAll(/([^{}]+)\{([^{}]*)\}/g)).filter(
      ([, selector]) => selector.includes(':focus-visible')
    );
    expect(focusRules.length).toBeGreaterThanOrEqual(4);
    for (const [, selector, body] of focusRules) {
      expect({ selector: selector.trim(), body }).not.toEqual(
        expect.objectContaining({ body: expect.stringContaining('--border-accent') })
      );
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
