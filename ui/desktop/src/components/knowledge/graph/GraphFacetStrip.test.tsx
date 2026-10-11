import { cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { Graph, GraphNode } from '../../../api/types.gen';
import { knowledgeCopy } from '../copy';
import { GraphFacetStrip } from './GraphFacetStrip';
import { EMPTY_FACETS } from './graphFacets';
import type { FacetState } from './graphFacets';
import { buildGraphModel } from './graphModel';

function node(id: string, extra: Partial<GraphNode> = {}): GraphNode {
  return { id, label: id, kind: 'entity', path: `knowledge/${id}.md`, ...extra } as GraphNode;
}

const typed: Graph = {
  nodes: [
    node('a', { node_type: 'Drug' }),
    node('b', { node_type: 'Drug' }),
    node('c', { node_type: 'Disease' }),
    // Three pages the base never gave a type. DR-28: a real state, not a hole.
    node('d'),
    node('e'),
    node('f'),
  ],
  edges: [],
};

const legacy: Graph = { nodes: [node('x'), node('y')], edges: [] };

function renderStrip(graph: Graph, active = false, facets: FacetState = EMPTY_FACETS) {
  const model = buildGraphModel(graph);
  return render(
    <GraphFacetStrip
      model={model}
      mode="light"
      facets={facets}
      onChange={vi.fn()}
      passing={active ? 2 : graph.nodes.length}
      total={graph.nodes.length}
      active={active}
    />
  );
}

describe('GraphFacetStrip', () => {
  /**
   * ⚠ **THIS TEST INVERTED, AND THE INVERSION IS R-09.**
   *
   * It used to assert that the readout and its undo sat OUTSIDE a horizontal
   * scroller while the pickers sat inside it — a fix for the measured failure
   * where one active facet put 769px of content in a 550px box and pushed both
   * off the right edge. That fix cured the symptom and left the pickers
   * themselves scrolling out of sight.
   *
   * There is now no scroller at all: the row degrades by priority into `More`
   * and then `Filters`, so every control stays reachable at every width. The
   * assertion is therefore that NOTHING in the strip scrolls sideways, which is
   * strictly stronger than the old one. Nothing in jsdom can measure overflow,
   * so this stays STRUCTURAL — the container-query steps themselves need a real
   * browser (see `.knowledge-harness`).
   */
  it('never scrolls sideways: no element in the strip is a horizontal scroller', () => {
    renderStrip(typed, true);
    const strip = screen.getByTestId('knowledge-graph-facets');

    expect(strip.querySelector('.overflow-x-auto')).toBeNull();
    expect(strip.className).not.toContain('overflow-x-auto');

    // The readout and its undo are still present and still in the strip.
    const readout = screen.getByText(/Showing 2 of 6/);
    const clear = screen.getByRole('button', { name: 'Clear filters' });
    for (const el of [readout, clear]) expect(strip.contains(el)).toBe(true);
  });

  /**
   * Spec 3.12: the strip takes the band height every strip in the app shares
   * (44px), its counts are sans tabular figures, and Clear explains itself in a
   * Tooltip rather than a native `title` with an em dash in it.
   */
  it('is a 44px band with sans counts and no native titles', () => {
    renderStrip(typed, true);
    const strip = screen.getByTestId('knowledge-graph-facets');
    expect(strip.className).toContain('h-chrome');
    expect(strip.querySelectorAll('[title]')).toHaveLength(0);
    expect(screen.getByText(/Showing 2 of 6/).className).not.toContain('font-mono');
  });

  /**
   * Below the 1400px step Clear is the bare icon (its label and the "Showing N of M" readout wait
   * for the room the Legend gives back), so the Tooltip is where the count lives. It must read
   * the sentence the copy file writes, with parentheses and no dash.
   */
  it('says how many pass in Clear’s Tooltip', async () => {
    renderStrip(typed, true);
    await userEvent.hover(screen.getByRole('button', { name: knowledgeCopy.graph.clearFilters }));
    const tip = await screen.findByRole('tooltip', {}, { timeout: 3000 });
    expect(tip).toHaveTextContent(knowledgeCopy.graph.clearFiltersHint(2, 6));
    expect(tip).toHaveTextContent('Clear filters (showing 2 of 6)');
  });

  /**
   * The collapsed controls exist at every width in the DOM — the container
   * queries decide which is VISIBLE — and each reports the count of what it
   * swallowed, so a filter the user cannot see is still reported.
   */
  it('offers a collapsed control for every step of the ladder', () => {
    renderStrip(typed, true);
    const more = screen.getByTestId('knowledge-graph-facet-more-collapsed');
    const all = screen.getByTestId('knowledge-graph-facet-filters-collapsed');
    expect(more.className).toContain('br-facet-more');
    expect(all.className).toContain('br-facet-all');

    // The inline facets carry their ladder step, so the CSS can fold them.
    expect(screen.getByTestId('knowledge-graph-facet-type').className).toContain('br-facet-core');
    expect(screen.getByTestId('knowledge-graph-facet-source').className).toContain(
      'br-facet-extra'
    );
  });

  /**
   * A filter is not a button, and after R-02 it is not one in the DOM either:
   * `data-engaged` is what `.br-facet` keys its selected fill off (the strong fill under
   * the selected wash, R-27), and it has to flip when a facet is actually engaged or the
   * fill is decoration.
   */
  it('marks an engaged facet so the fill can change, not just a badge', () => {
    // The strip is controlled, so this asserts the MAPPING from facet state to
    // the attribute rather than driving it through a click — which with a
    // `vi.fn()` onChange could never flip anything.
    renderStrip(typed);
    expect(screen.getByTestId('knowledge-graph-facet-type').getAttribute('data-engaged')).toBe(
      'false'
    );

    cleanup();
    renderStrip(typed, false, { ...EMPTY_FACETS, types: new Set(['Gene']) });
    const engaged = screen.getByTestId('knowledge-graph-facet-type');
    expect(engaged.getAttribute('data-engaged')).toBe('true');
    // …and the count rides on the control that is engaged.
    expect(within(engaged).getByText('1')).toBeInTheDocument();
  });

  it('says nothing about a filter until one is active', () => {
    renderStrip(typed, false);
    expect(screen.queryByText(/Showing/)).toBeNull();
    expect(screen.queryByRole('button', { name: 'Clear filters' })).toBeNull();
  });

  it('counts the Untyped row instead of hardcoding it to zero', async () => {
    renderStrip(typed);
    await userEvent.click(screen.getByTestId('knowledge-graph-facet-type'));

    const untyped = await screen.findByRole('option', { name: /Untyped/ });
    // Three pages carry no `node_type`. The row used to read "Untyped" with no
    // number beside it while every other row had one, which reads as "none".
    expect(within(untyped).getByText('3')).toBeInTheDocument();
  });

  it('counts every page of a wholly legacy base', async () => {
    renderStrip(legacy);
    await userEvent.click(screen.getByTestId('knowledge-graph-facet-type'));

    const untyped = await screen.findByRole('option', { name: /Untyped/ });
    expect(within(untyped).getByText('2')).toBeInTheDocument();
  });
});
