import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { ChangeKind } from '../../../api/types.gen';
import { ChangeKindChip } from './ChangeKindChip';
import { knowledgeCopy } from '../copy';

const ALL_KINDS: ChangeKind[] = ['ingest', 'link', 'flag', 'query', 'lint', 'restore', 'manual'];

/**
 * ⚠ There was no test file for `ChangeKindChip` or `ChangeLogDrawer` at all, so
 * the defect below was unguarded in BOTH directions: nothing stopped the status
 * hues coming back, and nothing would have caught them going in. This file is
 * the guard, and it lands with the fix (ui-spec §4.10).
 */
describe('ChangeKindChip', () => {
  // A KIND is a taxonomy, not a state. Painting `query` success-green and
  // `lint` warning-amber made a log where nothing had gone wrong read as a
  // mixture of successes and warnings.
  it('does not spend a status hue on a taxonomy', () => {
    render(
      <>
        {ALL_KINDS.map((kind) => (
          <ChangeKindChip key={kind} kind={kind} />
        ))}
      </>
    );

    for (const kind of ALL_KINDS) {
      const chip = screen
        .getByText(knowledgeCopy.changeKind[kind])
        .closest('[data-testid="change-kind-chip"]');
      expect(chip, `${kind} chip`).not.toBeNull();
      const className = chip!.className;
      if (kind === 'flag') {
        // The one exception, and the reason it is one: a flag genuinely IS a
        // problem marker.
        expect(className).toContain('text-text-danger');
      } else {
        expect(className, `${kind} must be neutral`).toContain('text-text-muted');
        expect(className, `${kind} must not carry a status hue`).not.toMatch(
          /text-text-(danger|success|warning|info|accent)/
        );
      }
    }
  });

  // The glyph is what distinguishes the seven kinds once the hue is gone, so it
  // has to actually differ per kind — and it must be decorative, because the
  // word is right beside it.
  it('distinguishes the kinds by a glyph that survives monochrome', () => {
    const seen = new Set<string>();
    for (const kind of ALL_KINDS) {
      const { container, unmount } = render(<ChangeKindChip kind={kind} />);
      const svg = container.querySelector('svg');
      expect(svg, `${kind} glyph`).not.toBeNull();
      expect(svg!.getAttribute('aria-hidden')).toBe('true');
      seen.add(svg!.outerHTML);
      unmount();
    }
    expect(seen.size).toBe(ALL_KINDS.length);
  });

  // Spec 3.12: sentence case, never the caps style.
  it('names each kind in sentence case', () => {
    for (const kind of ALL_KINDS) {
      const { unmount } = render(<ChangeKindChip kind={kind} />);
      const chip = screen.getByTestId('change-kind-chip');
      expect(chip).toHaveTextContent(knowledgeCopy.changeKind[kind]);
      expect(chip.className).not.toMatch(/text-caps|uppercase/);
      unmount();
    }
  });
});
