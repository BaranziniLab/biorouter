import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ loadRegistry: vi.fn() }));

vi.mock('./registry', async (importOriginal) => ({
  ...(await importOriginal<typeof import('./registry')>()),
  loadRegistry: mocks.loadRegistry,
}));
vi.mock('./installSkill', () => ({ installRegistrySkill: vi.fn() }));
vi.mock('../../toasts', () => ({ toastSuccess: vi.fn(), toastError: vi.fn() }));

import BrowseSkillsModal, { installButtonLabel } from './BrowseSkillsModal';

const skill = {
  id: 'scientific-research',
  name: 'scientific-research',
  category: 'Core' as const,
  // A real registry value: prose, not a token. See landing/registry.json.
  type: 'User-invocable · /scientific-research',
  description: 'Research workflows',
  tags: [],
  keywords: [],
  download: 'https://example.com/scientific-research.zip',
  filename: 'scientific-research.zip',
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.loadRegistry.mockResolvedValue({
    registry: { version: 1, skills: [skill] },
    live: true,
    fetchedAt: '2026-09-02T00:00:00Z',
  });
});

/// `skill.type` is an English phrase, not a machine token. The real registry
/// ships values like "5 skills · auto-applied" and
/// "User-invocable · /scientific-research" — and this span sits inline, on the
/// same row, beside `skill.name` in the body font. Monospace here was the
/// reverse defect: prose set in the code face.
///
/// jsdom never runs Tailwind, so asserting a computed font would pass whatever
/// the class says. This asserts the CLASS, and walks the ancestors because
/// `font-mono` on a parent is inherited.
describe('BrowseSkillsModal — a classification phrase is prose, not a token', () => {
  it('sets the skill type in the body font, not monospace', async () => {
    render(<BrowseSkillsModal onClose={vi.fn()} onInstalled={vi.fn()} installedIds={new Set()} />);

    const type = await screen.findByText(skill.type);
    expect(type.className).not.toMatch(/font-mono/);
    for (let node = type.parentElement; node; node = node.parentElement) {
      expect(node.className ?? '').not.toMatch(/font-mono/);
      if (node.tagName === 'BODY') break;
    }
  });
});

/// The button reads "Install  skills" — two spaces — whenever nothing is
/// selected, which is the state the dialog OPENS in. The old template put the
/// count between two literal spaces (`` `Install ${n > 0 ? n : ''} skill…` ``),
/// so the empty substitution collapsed to nothing and left the pair behind.
///
/// Tested on the pure label rather than only through a render: the arithmetic
/// is a function of one number, and a threshold you can exercise only by
/// mounting a component is one nobody re-tests.
describe('BrowseSkillsModal — the install label has one space between words', () => {
  it.each([
    [0, 'Install skills'],
    [1, 'Install 1 skill'],
    [2, 'Install 2 skills'],
    [3, 'Install 3 skills'],
  ])('reads correctly with %i selected', (count, expected) => {
    expect(installButtonLabel(count)).toBe(expected);
  });

  it('never emits a double space at any count', () => {
    for (let n = 0; n <= 12; n += 1) {
      expect(installButtonLabel(n)).not.toMatch(/ {2}/);
      expect(installButtonLabel(n).trim()).toBe(installButtonLabel(n));
    }
  });

  /// The one shape the user actually hits first, asserted through the real
  /// DOM so the helper cannot be correct while the button ignores it.
  it('renders "Install skills" in the freshly-opened dialog', async () => {
    render(<BrowseSkillsModal onClose={vi.fn()} onInstalled={vi.fn()} installedIds={new Set()} />);

    const button = await screen.findByRole('button', { name: 'Install skills' });
    expect(button).toBeDisabled();
    // `textContent` rather than the accessible name, which normalises runs of
    // whitespace and would report the bug as fixed while it was still there.
    expect(button.textContent).toBe('Install skills');
  });
});
