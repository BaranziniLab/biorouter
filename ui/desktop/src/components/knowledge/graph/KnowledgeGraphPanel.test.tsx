import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { KnowledgeGraphPanel } from './KnowledgeGraphPanel';

// The canvas needs a real 2D context and a layout engine; the error state draws neither.
vi.mock('./ForceGraphCanvas', () => ({ ForceGraphCanvas: () => null }));

/**
 * R-27 on the graph pane's own ground. The canvas paints `--background-muted`, which is the
 * value `--background-fill` has (1.00:1), so a filled control sitting directly on it rests on
 * `--background-fill-strong` and hovers one fill deeper, `--background-strong` (design.md 9.11).
 * The canvas says so once, for every filled control on it (`.br-knowledge-recessed`, which
 * `knowledgeFill.browser.test.tsx` measures in Chromium), so the pane's one such control, the
 * error state's retry, is the plain filled secondary with no ground of its own. jsdom runs no
 * Tailwind, so this pins the classes the rule is written in.
 */
describe('KnowledgeGraphPanel on its muted ground', () => {
  it('lets the canvas rest the retry button on the strong fill', async () => {
    const onRefresh = vi.fn();
    render(
      <KnowledgeGraphPanel
        kbId="kb-1"
        graph={null}
        loading={false}
        error="The graph could not be read."
        onRefresh={onRefresh}
        previewSha={null}
        onClearPreview={() => undefined}
      />
    );

    const retry = screen.getByRole('button', { name: 'Try again' });
    // The filled secondary: the control ground and the hover step come from its classes.
    expect(retry.className).toMatch(/\bbr-fill-rest\b/);
    expect(retry.className).toMatch(/\bbr-fill-hover\b/);
    // Not a ground of its own: a `bg-*` here would beat the layered fill and the hover with it.
    expect(retry.className).not.toMatch(/\bbg-background-/);
    // The recessed ground is what moves the rest to the strong step and the hover one deeper.
    expect(retry.closest('.br-knowledge-recessed')).not.toBeNull();

    await userEvent.click(retry);
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });
});
