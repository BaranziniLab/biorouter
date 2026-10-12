import { describe, expect, it, vi } from 'vitest';
import { render } from '@testing-library/react';
import ToolCallWithResponse from './ToolCallWithResponse';
import type { ToolRequestMessageContent, ToolResponseMessageContent } from '../types/message';

/**
 * The leading glyph a transcript row draws, read off the rendered row.
 * `toolGlyph.test.ts` pins the ladder; this pins that the row asks it (V2-R5-3:
 * a chat saved before Biorouter Copilot became the one desktop surface drew
 * "Ran screen capture" and "Listed windows" with the code glyph).
 */
const noopOpenArtifact = vi.fn();

function row(name: string, args: Record<string, unknown> = {}, error = false) {
  const id = `glyph-${name}`;
  const request = {
    type: 'toolRequest',
    id,
    toolCall: { status: 'success', value: { name, arguments: args } },
  } as ToolRequestMessageContent;
  const response = {
    type: 'toolResponse',
    id,
    toolResult: {
      status: 'success',
      value: { content: [{ type: 'text', text: error ? 'No display' : 'ok' }], isError: error },
    },
  } as ToolResponseMessageContent;
  return render(
    <ToolCallWithResponse
      isCancelledMessage={false}
      toolRequest={request}
      toolResponse={response}
      onOpenArtifact={noopOpenArtifact}
    />
  );
}

/** The class list of the first glyph in the row's header button. */
function leadingGlyph(container: HTMLElement): SVGElement {
  const button = container.querySelector('button');
  expect(button, 'the row has a header button').not.toBeNull();
  const svg = button!.querySelector('svg');
  expect(svg, 'the header draws a glyph').not.toBeNull();
  return svg as unknown as SVGElement;
}

describe('a legacy developer capture row draws the Copilot glyph', () => {
  for (const name of ['developer__screen_capture', 'developer__list_windows']) {
    it(`${name}, finished`, () => {
      const { container } = row(name);
      const glyph = leadingGlyph(container);
      expect(glyph.classList.contains('br-icon-copilot'), name).toBe(true);
      expect(glyph.classList.contains('lucide-code'), name).toBe(false);
    });
  }

  it('a failed capture draws it too', () => {
    const { container } = row('developer__screen_capture', {}, true);
    expect(leadingGlyph(container).classList.contains('br-icon-copilot')).toBe(true);
  });

  it('sits beside a computer control row with the same figure', () => {
    const legacy = row('developer__screen_capture').container;
    const control = row('computercontroller__computer_control').container;
    expect(leadingGlyph(legacy).innerHTML).toBe(leadingGlyph(control).innerHTML);
  });

  it('leaves the rest of the Developer toolbox alone', () => {
    const { container } = row('developer__shell', { command: 'ls' });
    expect(leadingGlyph(container).classList.contains('br-icon-copilot')).toBe(false);
  });
});
