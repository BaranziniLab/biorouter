import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import userEvent from '@testing-library/user-event';
import { readFileSync } from 'node:fs';
import { MessageActions, MessageMetaAction } from './MessageMeta';

// vitest runs with `ui/desktop` as the root.
const css = readFileSync('src/components/message-meta.css', 'utf-8');

describe('MessageActions (the floating cluster)', () => {
  it('sets the time at `supporting`, not at message size', () => {
    render(<MessageActions timestamp="3:14 PM" />);
    // 12px metadata, never the 14px the message body uses, and tabular so the
    // digits do not shift as the minute changes.
    expect(screen.getByText('3:14 PM')).toHaveClass(
      'text-supporting',
      'text-text-muted',
      'tabular-nums'
    );
  });

  it('holds the time first, then the actions, in one cluster', () => {
    const { container } = render(
      <MessageActions align="end" timestamp="3:14 PM">
        <button type="button">Copy</button>
      </MessageActions>
    );
    const cluster = container.querySelector('[data-message-meta="end"]')!;
    expect(cluster).toHaveClass('br-message-actions');
    expect(cluster.firstElementChild).toHaveTextContent('3:14 PM');
    expect(cluster.lastElementChild).toBe(screen.getByRole('button', { name: 'Copy' }));
  });

  it('reserves nothing in the flow: it floats, and only its opacity changes', () => {
    // jsdom applies no stylesheet, so the rule is read where the browser reads it.
    const rule = /\.br-message-actions \{([^}]*)\}/.exec(css)?.[1] ?? '';
    expect(rule).toContain('position: absolute;');
    // Above an answer it rises 8px, the smallest gap an answer has, so it never
    // lands on the block above; above the user bubble it rises 20px and ends in
    // the bubble's top padding, clear of the words (vision V2-12).
    expect(rule).toContain('top: -8px;');
    expect(
      /\.br-message-actions-anchor > \.br-message-actions \{([^}]*)\}/.exec(css)?.[1]
    ).toContain('top: -20px;');
    expect(rule).toContain('opacity: 0;');
    expect(rule).toContain('box-shadow: var(--shadow-popover), var(--inset-hairline);');
    expect(rule).toContain('transition: opacity var(--dur-fast-min) var(--ease-out);');
    expect(rule).not.toMatch(/transform|translate/);
    // Shown on hover, on focus inside the message, and while its menu is open.
    expect(css).toContain('.br-message-actions-scope:hover .br-message-actions');
    expect(css).toContain('.br-message-actions-scope:focus-within .br-message-actions');
    expect(css).toContain(".br-message-actions:has([data-state='open'])");
    // Always shown, in the flow, without a hover pointer.
    expect(css).toMatch(/@media \(hover: none\) \{[\s\S]*?position: static;[\s\S]*?opacity: 1;/);
  });

  it('keeps an edge under forced colours, where box shadows are dropped', () => {
    // The cluster's whole edge is a shadow; without a border it would float
    // edgeless over the words.
    const forced = /@media \(forced-colors: active\) \{([\s\S]*?)\n\}/.exec(css)?.[1] ?? '';
    expect(forced).toMatch(/\.br-message-actions \{[^}]*border: 1px solid CanvasText;/);
    expect(forced).toMatch(/\.br-message-actions button \{[^}]*border: 1px solid ButtonText;/);
  });
});

describe('MessageMetaAction (an action about the content, in the flow)', () => {
  it('does not fire when disabled', async () => {
    const onClick = vi.fn();
    render(
      <MessageMetaAction icon={<svg />} disabled onClick={onClick}>
        Show more
      </MessageMetaAction>
    );
    await userEvent.click(screen.getByRole('button', { name: 'Show more' }));
    expect(onClick).not.toHaveBeenCalled();
  });
});
