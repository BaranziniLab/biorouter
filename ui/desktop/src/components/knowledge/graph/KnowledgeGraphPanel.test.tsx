import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { knowledgeCopy } from '../copy';
import { KnowledgeGraphPanel } from './KnowledgeGraphPanel';

// The canvas needs a real 2D context and a layout engine; the error state draws neither.
vi.mock('./ForceGraphCanvas', () => ({ ForceGraphCanvas: () => null }));

/**
 * R-27 on the graph pane's own ground. The canvas paints `--background-muted`, which is the
 * value `--background-fill` has (1.00:1), so a filled control sitting directly on it rests on
 * `--background-fill-strong` and hovers one fill deeper, `--background-strong` (design.md 9.11).
 * The canvas says so once, for every filled control on it (the shared `.br-ground-recessed`, which
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
    expect(retry.closest('.br-ground-recessed')).not.toBeNull();

    await userEvent.click(retry);
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });
});

/**
 * V3-R3-6: the empty tile is read at every pane width, and the pane changes what its Sources
 * column is. Below the 860px step it is a tab, beside the graph it is a column that draws no
 * label, and the button the old line told people to press reads "Digest staged sources" in both.
 * So the line may name neither a container nor a button label that is not there.
 */
describe('KnowledgeGraphPanel empty state', () => {
  const renderEmpty = () =>
    render(
      <KnowledgeGraphPanel
        kbId="kb-1"
        graph={{ nodes: [], edges: [] }}
        loading={false}
        error={null}
        onRefresh={() => undefined}
        previewSha={null}
        onClearPreview={() => undefined}
      />
    );

  it('says what to do without naming where the Sources column is', () => {
    renderEmpty();

    expect(
      screen.getByRole('heading', { name: knowledgeCopy.graph.emptyTitle })
    ).toBeInTheDocument();
    const hint = screen.getByText(knowledgeCopy.graph.emptyHint);
    expect(hint).toBeInTheDocument();
    // A container name is true at one step and false at the other.
    expect(hint.textContent).not.toMatch(/\b(rail|tab|column|pane|panel|sidebar|sources)\b/i);
    // The old line told people to press "Digest", which is no button's name.
    expect(hint.textContent).not.toMatch(/\bpress\b/i);
  });

  it('uses the verbs the controls use', () => {
    // The dropzone says "stage" and the primary button says "Digest staged sources".
    expect(knowledgeCopy.ingest.dropLabel).toMatch(/\bstage\b/i);
    expect(knowledgeCopy.graph.emptyHint).toMatch(/\bstage\b/i);
    expect(knowledgeCopy.graph.emptyHint).toMatch(/\bdigest\b/i);
  });
});
