import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { DECISION_HISTORY_LINK_COPY, DecisionHistoryLink } from './DecisionHistoryLink';

/** A focus the Tab key moved shows a label tooltip (Tooltip's Tab gate). */
function tabTo(element: HTMLElement) {
  fireEvent.keyDown(document.activeElement ?? document.body, { key: 'Tab' });
  act(() => element.focus());
}

describe('DecisionHistoryLink (F-13: one link, one look)', () => {
  it('reads "Decision history", is named for what it opens, and its tooltip says the same', async () => {
    const onOpen = vi.fn();
    const label = 'Open this decide call in the decision history';
    render(<DecisionHistoryLink label={label} onOpen={onOpen} />);
    const link = screen.getByRole('button', { name: label });
    expect(link).toHaveTextContent(/^Decision history$/);
    expect(link).toHaveClass('br-decision-link');
    expect(link).not.toHaveClass('br-decision-link--line');
    // The name contains the visible words, so label-in-name holds.
    expect(label.toLowerCase()).toContain(DECISION_HISTORY_LINK_COPY.text.toLowerCase());
    tabTo(link);
    expect((await screen.findAllByText(label)).length).toBeGreaterThan(0);
    fireEvent.click(link);
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it('is the link variant, start-aligned, with no fill: authored in decisionsPanel.css', () => {
    const source = readFileSync(join(__dirname, 'DecisionHistoryLink.tsx'), 'utf8');
    expect(source).toMatch(/variant="link"/);
    expect(source).toMatch(/size="xs"/);
    const css = readFileSync(join(__dirname, 'decisionsPanel.css'), 'utf8');
    const rule = css.match(/\n\.br-decision-link \{([^}]*)\}/)?.[1] ?? '';
    expect(rule).toContain('align-self: flex-start');
    expect(rule).toContain('padding-inline: 0');
    expect(rule).not.toMatch(/background/);
    expect(css).toMatch(/\.br-decision-link--line \{[^}]*color: var\(--text-default\)/);
  });

  it('in its line form, shows its children in quiet ink and keeps the name it was given', () => {
    render(
      <DecisionHistoryLink label="Open Done with shell in the decision history" onOpen={vi.fn()}>
        Done with shell: checked 3 times, fired twice
      </DecisionHistoryLink>
    );
    const link = screen.getByRole('button', {
      name: 'Open Done with shell in the decision history',
    });
    expect(link).toHaveTextContent('Done with shell: checked 3 times, fired twice');
    expect(link).toHaveClass('br-decision-link', 'br-decision-link--line');
  });
});

/**
 * Source guards over every file in this folder. The decision chat's card
 * (`DecisionCard.tsx` and its `decisionsChat.css`) belongs to another package in this
 * round; it is listed as PENDING, and the last test fails once it has moved, so the
 * exemption is removed with the move instead of staying behind.
 */
const PENDING = new Set(['DecisionCard.tsx', 'decisionsChat.css']);
const FILES = readdirSync(__dirname).filter(
  (name) => /\.(tsx?|css)$/.test(name) && !/\.test\.tsx?$/.test(name)
);
const read = (name: string) => readFileSync(join(__dirname, name), 'utf8');

describe('the old link styles are gone', () => {
  it('no file writes the retired .br-dh-link', () => {
    for (const name of FILES) expect(read(name), name).not.toContain('br-dh-link');
  });

  it('no file writes .br-decision-card__history', () => {
    for (const name of FILES.filter((file) => !PENDING.has(file))) {
      expect(read(name), name).not.toContain('br-decision-card__history');
    }
  });

  it('no file but DecisionHistoryLink draws "Decision history" as link text', () => {
    // A menu item or a footer button may say "Decision history"; a link-styled
    // button may not (it is this component's job).
    for (const name of FILES.filter(
      (file) => file !== 'DecisionHistoryLink.tsx' && !PENDING.has(file)
    )) {
      const source = read(name);
      const linkButtons = source.match(/<Button[^>]*variant="link"[\s\S]*?<\/Button>/g) ?? [];
      for (const button of linkButtons) {
        expect(button, name).not.toMatch(/Decision history|copy\.history|\.historyShort/);
      }
    }
  });

  it('still lists only files that have not moved yet', () => {
    for (const name of PENDING) {
      expect(read(name), `${name} has moved: remove it from PENDING`).toContain(
        'br-decision-card__history'
      );
    }
  });
});
