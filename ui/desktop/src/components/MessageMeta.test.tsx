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
    // It rises 26px, so its 28px reach ends 2px into the message, above an
    // answer's glyphs and inside the bubble's top padding: never on the words
    // of the message it acts on (vision V2-12, re-reported in round 2 when it
    // rose 8px over an answer's first line).
    expect(rule).toContain('top: -26px;');
    // A bubble 8px under another rises 18px: paddings and gap only.
    expect(
      /\[data-gap='block'\]\[data-role='user'\] \.br-message-actions-anchor > \.br-message-actions \{([^}]*)\}/.exec(
        css
      )?.[1]
    ).toContain('top: -18px;');
    // Beside a provenance chip it sits on the chip's line, to its left.
    expect(/\.br-message-actions-chip > \.br-message-actions \{([^}]*)\}/.exec(css)?.[1]).toMatch(
      /right: calc\(100% \+ 8px\);/
    );
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

  it('ends the tool row an answer rises into short of the cluster (vision V2-12, round 3)', () => {
    const selector =
      '[data-actions-below] .biorouter-message-tool:last-child .br-transcript-row-line';
    const escaped = selector.replace(/[[\].:-]/g, (c) => `\\${c}`);
    const rules = Array.from(
      css.matchAll(new RegExp(`${escaped} \\{([^}]*)\\}`, 'g')),
      (match) => match[1]
    );
    // The widest cluster (183px) and 8px of air; in the flow (no hover
    // pointer) it rises into nothing and the row keeps its width.
    expect(rules).toHaveLength(2);
    expect(rules[0]).toContain('padding-inline-end: 192px;');
    expect(rules[1]).toContain('padding-inline-end: 0;');
    const noHover = /@media \(hover: none\) \{([\s\S]*?)\n\}/.exec(css)?.[1] ?? '';
    expect(noHover).toContain(selector);
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
