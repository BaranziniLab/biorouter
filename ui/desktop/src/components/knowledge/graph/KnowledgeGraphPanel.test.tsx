import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { KnowledgeGraphPanel } from './KnowledgeGraphPanel';

// The canvas needs a real 2D context and a layout engine; the error state draws neither.
vi.mock('./ForceGraphCanvas', () => ({ ForceGraphCanvas: () => null }));

/**
 * R-27 on the graph pane's own ground. The canvas paints `--background-muted`, which is the
 * value `--background-fill` has (1.00:1), so a filled control sitting directly on it rests on
 * `--background-fill-strong` and keeps a transparent border for forced colours (design.md 9.11).
 * The pane's one such control is the error state's retry. jsdom runs no Tailwind, so this pins
 * the classes the rule is written in.
 */
describe('KnowledgeGraphPanel on its muted ground', () => {
  it('rests the retry button on the strong fill with a transparent border', async () => {
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
    const classes = retry.className.split(/\s+/);
    expect(classes).toContain('bg-background-fill-strong');
    expect(classes).toContain('border-transparent');
    // Not the resting fill, and not the variant's own ground, which the override replaces.
    expect(classes).not.toContain('bg-background-fill');
    expect(classes).not.toContain('bg-background-medium');

    await userEvent.click(retry);
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });
});
