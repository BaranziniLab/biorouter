import { useRef } from 'react';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { QuotedTextSelection, useTextSelection } from './QuotedTextSelection';
import { onQuotedText } from '../utils/quotedText';

function Fixture({ text, sourceKey = 'response' }: { text: string; sourceKey?: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const selection = useTextSelection(ref, sourceKey);
  return (
    <QuotedTextSelection
      source={{ sessionId: 'selection-test', title: sourceKey }}
      selection={selection}
    >
      <section>
        {text ? (
          <div ref={ref} data-testid="source">
            {text}
          </div>
        ) : null}
      </section>
    </QuotedTextSelection>
  );
}

function selectText() {
  const range = document.createRange();
  range.selectNodeContents(screen.getByTestId('source'));
  const selection = document.getSelection()!;
  selection.removeAllRanges();
  selection.addRange(range);
  fireEvent(document, new Event('selectionchange'));
}

describe('response selection menu', () => {
  it('subscribes before streamed text mounts and keeps Copy alongside quote actions', async () => {
    const receive = vi.fn();
    const off = onQuotedText('selection-test', receive);
    const { rerender } = render(<Fixture text="" />);
    rerender(<Fixture text={'Exact "text"\nsecond line'} />);
    act(selectText);
    fireEvent.contextMenu(screen.getByTestId('source'));
    expect(await screen.findByRole('menuitem', { name: 'Copy' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Ask about it' }));
    expect(receive).toHaveBeenCalledWith({
      source: { sessionId: 'selection-test', title: 'response' },
      text: 'Exact "text"\nsecond line',
    });
    off();
  });
  it('clears the selection when the source changes', () => {
    const { rerender } = render(<Fixture text="Old text" />);
    act(selectText);
    rerender(<Fixture text="New text" sourceKey="new" />);
    fireEvent.contextMenu(screen.getByTestId('source'));
    expect(screen.queryByRole('menuitem')).not.toBeInTheDocument();
  });
});
