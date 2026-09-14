import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

/**
 * The kind glyph's privacy statement when the tier is NOT known.
 *
 * Measured 2026-09-14 on the 1.90.4 release candidate (main 1038a113): a
 * private chat's subagent tabs drew `data-privacy="public"` for ~0.5 s after a
 * Settings round-trip (3.2 s on the tester's machine) and ~1.5 s after a
 * reload, then flipped to private once their `metadata_only` reads answered.
 * The tier maps upstream already kept "no opinion" apart from Public; this
 * component folded the two back together with `effectiveTier ?? 'public'`.
 *
 * NOTE: like the other glyph suites, this file never names the badge
 * component — Task 27's gate greps src/components for that name.
 */

let tiersEnabled = true;
vi.mock('../ConfigContext', () => ({
  usePrivacyTiersEnabled: () => tiersEnabled,
}));

import { ChatKindIcon } from './ChatKindIcon';

const SUBAGENT = { name: 'Subagent: Reply with ALPHA', session_type: 'sub_agent' };
const PLAIN = { name: 'Cohort query' };

describe('ChatKindIcon — a tier nobody has read is not Public', () => {
  it('draws a subagent with no tier as unknown, not public', () => {
    tiersEnabled = true;
    render(<ChatKindIcon session={SUBAGENT} />);
    const glyph = screen.getByTestId('chat-kind-icon');
    expect(glyph).not.toHaveAttribute('data-privacy', 'public');
    expect(glyph).toHaveAttribute('data-privacy', 'unknown');
    expect(glyph).toHaveAttribute('data-chat-kind', 'subagent');
    expect(glyph.getAttribute('aria-label')).toBe('Sub-agent, privacy not yet known');
    // Not the private ink either: unknown claims nothing in either direction.
    expect(glyph.getAttribute('class') ?? '').not.toContain('text-text-accent');
  });

  it('draws a plain chat with no tier as unknown, in the unmarked shape', () => {
    tiersEnabled = true;
    const { unmount } = render(<ChatKindIcon session={PLAIN} tier={null} />);
    const unknown = screen.getByTestId('chat-kind-icon');
    expect(unknown).toHaveAttribute('data-privacy', 'unknown');
    expect(unknown.getAttribute('aria-label')).toBe('Chat, privacy not yet known');
    const unknownShape = unknown.innerHTML;
    unmount();

    // Same figure as public — a padlock is a claim, and this state makes none.
    render(<ChatKindIcon session={PLAIN} tier="public" />);
    expect(screen.getByTestId('chat-kind-icon').innerHTML).toBe(unknownShape);
  });

  it('still says public, promptly, for a chat a source has read as public', () => {
    tiersEnabled = true;
    render(<ChatKindIcon session={SUBAGENT} tier="public" />);
    const glyph = screen.getByTestId('chat-kind-icon');
    expect(glyph).toHaveAttribute('data-privacy', 'public');
    expect(glyph.getAttribute('aria-label')).toBe('Sub-agent');
  });

  it('still marks a private chat private', () => {
    tiersEnabled = true;
    render(<ChatKindIcon session={SUBAGENT} tier="private" />);
    const glyph = screen.getByTestId('chat-kind-icon');
    expect(glyph).toHaveAttribute('data-privacy', 'private');
    expect(glyph.getAttribute('aria-label')).toBe('Sub-agent, private');
    expect(glyph.getAttribute('class') ?? '').toContain('text-text-accent');
  });

  /**
   * With the master switch off the glyph stands down entirely (DR-15). It used
   * to stand down to `public`, which is a tier statement too.
   */
  it('makes no tier statement at all when privacy tiers are switched off', () => {
    tiersEnabled = false;
    for (const tier of ['private', 'public', undefined] as const) {
      const { unmount } = render(<ChatKindIcon session={SUBAGENT} tier={tier} />);
      const glyph = screen.getByTestId('chat-kind-icon');
      expect(glyph).toHaveAttribute('data-privacy', 'off');
      expect(glyph.getAttribute('aria-label')).toBe('Sub-agent');
      expect(glyph.getAttribute('class') ?? '').not.toContain('text-text-accent');
      unmount();
    }
    tiersEnabled = true;
  });
});

/**
 * The unknown state must LOOK different from Public, not just carry a different
 * attribute. jsdom never loads `main.css` and computes no opacity, so the rule
 * is asserted where it lives — the same reason `styles/composerFocus.test.ts`
 * reads the stylesheet. It is authored CSS keyed on `data-privacy`, never a
 * Tailwind class at the call site: a newly written utility can silently fail to
 * generate under BIOROUTER_NO_HMR, and this state must not depend on that.
 */
describe('the unknown glyph is visibly not the public one', () => {
  const CSS = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');
  const RULE = /\.br-chat-kind-icon\[data-privacy=['"]unknown['"]\]\s*\{([^}]*)\}/;

  it('is dimmed by an authored rule keyed on the attribute', () => {
    const body = CSS.match(RULE)?.[1];
    expect(body).toBeTruthy();
    const opacity = Number(body!.match(/opacity:\s*([0-9.]+)/)?.[1]);
    expect(opacity).toBeGreaterThan(0);
    expect(opacity).toBeLessThanOrEqual(0.6);
  });

  it('hangs the class the rule selects on every glyph', () => {
    render(<ChatKindIcon session={SUBAGENT} />);
    expect(screen.getByTestId('chat-kind-icon').getAttribute('class') ?? '').toContain(
      'br-chat-kind-icon'
    );
  });

  it('dims nothing that has a known tier', () => {
    expect(CSS).not.toMatch(/\.br-chat-kind-icon\[data-privacy=['"](public|private|off)['"]\]/);
  });
});
