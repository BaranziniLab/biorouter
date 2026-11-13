import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

import { TruncatedLine } from './truncated-line';

const LINE =
  'Cypher queries on the SPOKE biomedical knowledge graph: diseases, genes, proteins, drugs and pathways.';

/** jsdom lays nothing out; give every element the widths of a line that does or does not fit. */
function layOut(scrollWidth: number, clientWidth: number) {
  vi.spyOn(HTMLElement.prototype, 'scrollWidth', 'get').mockReturnValue(scrollWidth);
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(clientWidth);
}

afterEach(() => vi.restoreAllMocks());

/**
 * Owner, 2026-11-13: a row's one-line description must not hide its end with no way to read it.
 * The tooltip that holds the whole line opens only while the line is cut off.
 */
describe('TruncatedLine', () => {
  it('is one line, and a cut-off line is a Tab stop whose tooltip holds the whole of it', async () => {
    layOut(600, 200);
    const user = userEvent.setup();
    render(<TruncatedLine text={LINE} className="text-supporting" />);

    const line = screen.getByText(LINE);
    expect(line).toHaveClass('truncate', 'text-supporting');
    expect(line).toHaveAttribute('tabindex', '0');

    await user.tab();
    expect(line).toHaveFocus();
    expect(await screen.findByRole('tooltip')).toHaveTextContent(LINE);
  });

  it('leaves a line that fits out of the tab order and opens nothing over it', async () => {
    layOut(200, 200);
    const user = userEvent.setup();
    render(<TruncatedLine text="Fits" />);

    expect(screen.getByText('Fits')).not.toHaveAttribute('tabindex');
    await user.hover(screen.getByText('Fits'));
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('takes no Tab stop when it sits in a row that is already the control', () => {
    layOut(600, 200);
    render(
      <label>
        <input type="checkbox" />
        <TruncatedLine text={LINE} focusable={false} />
      </label>
    );

    expect(screen.getByText(LINE)).not.toHaveAttribute('tabindex');
  });
});
