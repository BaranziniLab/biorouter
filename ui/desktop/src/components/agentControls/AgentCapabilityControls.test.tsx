import { readFileSync } from 'node:fs';
import path from 'node:path';
import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { BrowserControl } from '../browser/BrowserControl';
import { CopilotControl } from '../copilot/CopilotControl';
import { AgentCapabilityControls } from './AgentCapabilityControls';

const mocks = vi.hoisted(() => ({
  copilot: vi.fn(),
  browser: vi.fn(),
  CopilotNotApplicable: class CopilotNotApplicable extends Error {},
  BrowserNotApplicable: class BrowserNotApplicable extends Error {},
}));
vi.mock('../copilot/copilotApi', () => ({
  copilotStatus: mocks.copilot,
  copilotDecision: vi.fn(),
  copilotSetup: vi.fn(),
  CopilotNotApplicable: mocks.CopilotNotApplicable,
}));
vi.mock('../browser/browserApi', () => ({
  browserStatus: mocks.browser,
  allowBrowser: vi.fn(),
  stopBrowser: vi.fn(),
  BrowserNotApplicable: mocks.BrowserNotApplicable,
}));
vi.mock('../../utils/surface', () => ({ isBrowserSurface: () => false }));

/** A Copilot approval waiting in chat `s` (the "Bundled runtime missing" card of the report). */
const copilotAsks = {
  session_id: 's',
  provider: 'versa_azure',
  model: 'gpt-5.5',
  destination: 'https://api.example',
  target: 'this-mac',
  disclosure: 'Allow Biorouter to view and control this computer?',
  state: 'approval_required',
  challenge_id: 'c1',
  public_model: false,
  handoff_required: false,
  requested: true,
  enabled: true,
  activity_id: 'a1',
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.copilot.mockResolvedValue(copilotAsks);
  mocks.browser.mockResolvedValue({ available: false, session_id: 's' });
});

const copilotCards = () => screen.queryAllByRole('region', { name: 'Biorouter Copilot' });

/**
 * M2 of the 2026-11-20 fix round: during a Copilot request several identical approval cards
 * stacked above the composer, and Cancel did not take them away until the chat was reloaded.
 *
 * The cause was the composer's slot in BaseChat: Biorouter Copilot's control and the Biorouter
 * Browser's were siblings with the SAME key, `sessionId`. Whenever a sibling before them is
 * absent (a notice that is not shown, which is nearly always), React reconciles that slot by key;
 * two children under one key collide in its map, so the old Copilot control is neither reused nor
 * removed. Every re-render of the chat left the previous Copilot control mounted, detached from
 * React, its DOM frozen on whatever it last drew: a pending card, whose Cancel can no longer
 * re-render it.
 */
describe('the agent capability controls above the composer (M2)', () => {
  it('reproduces the stacking with the two controls under one key (the old BaseChat slot)', async () => {
    // The shape BaseChat had: direct siblings of the composer, after a notice that is absent.
    const ready = true;
    const twoUnderOneKey = (notice: boolean) => (
      <div>
        {notice && <p>notice</p>}
        <p>pinned model note</p>
        {ready && <CopilotControl key="s" sessionId="s" />}
        {ready && <BrowserControl key="s" sessionId="s" />}
        <textarea aria-label="composer" />
      </div>
    );
    // React reports the duplicate key; the test is about what follows from it.
    const quiet = vi.spyOn(console, 'error').mockImplementation(() => {});
    const view = render(twoUnderOneKey(false));
    await waitFor(() => expect(copilotCards()).toHaveLength(1));
    for (const notice of [false, true, false, false]) {
      view.rerender(twoUnderOneKey(notice));
      await waitFor(() => expect(copilotCards().length).toBeGreaterThan(0));
    }
    await waitFor(() => expect(copilotCards().length).toBeGreaterThan(1));
    quiet.mockRestore();
    view.unmount();
  });

  it('keeps exactly one Copilot card however often the chat re-renders', async () => {
    const ready = true;
    const slot = (notice: boolean) => (
      <div>
        {notice && <p>notice</p>}
        <p>pinned model note</p>
        {ready && <AgentCapabilityControls key="s" sessionId="s" />}
        <textarea aria-label="composer" />
      </div>
    );
    const view = render(slot(false));
    await waitFor(() => expect(copilotCards()).toHaveLength(1));
    for (const notice of [false, true, false, false, true]) {
      view.rerender(slot(notice));
      await waitFor(() => expect(copilotCards().length).toBeGreaterThan(0));
    }
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(copilotCards()).toHaveLength(1);
    view.unmount();
    expect(copilotCards()).toHaveLength(0);
  });

  it('is the one place BaseChat mounts the two controls, under one key', () => {
    const source = readFileSync(
      path.join(process.cwd(), 'src', 'components', 'BaseChat.tsx'),
      'utf8'
    );
    expect(source).toMatch(
      /<AgentCapabilityControls key=\{sessionId\} sessionId=\{sessionId\} \/>/
    );
    expect(source).not.toMatch(/<CopilotControl\b/);
    expect(source).not.toMatch(/<BrowserControl\b/);
  });
});
