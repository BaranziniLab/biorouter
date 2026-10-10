import * as React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';
import BuiltInBadge from '../ui/BuiltInBadge';
import { PickerPanel, PickerRow } from './PickerPanel';
import { TOOLS_COPY } from './copy';

/**
 * The picker shell on its own (spec 3.7): one combobox over one listbox, the same
 * for extensions, skills and knowledge bases. The lists are tested with their data
 * in `BottomMenu*Selection*.test.tsx`; this is the contract they all share.
 */
beforeAll(() => {
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
});
afterAll(() => vi.unstubAllGlobals());

type Row = { name: string; on: boolean; disabled?: boolean; note?: string; description?: string };

function Harness({
  initial,
  onBulk = () => {},
  onToggled = () => {},
}: {
  initial: Row[];
  onBulk?: () => void;
  onToggled?: (name: string) => void;
}) {
  const [rows, setRows] = React.useState(initial);
  const [query, setQuery] = React.useState('');
  return (
    <PickerPanel
      listLabel="Extensions"
      searchPlaceholder="Search extensions…"
      query={query}
      onQueryChange={setQuery}
      scopeLabel={TOOLS_COPY.scope.thisChat}
      scopeHelp="Changes apply to this chat only."
      bulk={{ label: 'Disable all (2)', onClick: onBulk }}
      emptyText="No extensions found"
    >
      {rows
        .filter((row) => row.name.includes(query))
        .map((row) => (
          <PickerRow
            key={row.name}
            name={row.name}
            checked={row.on}
            disabled={row.disabled}
            note={row.note}
            description={row.description}
            onToggle={() => {
              onToggled(row.name);
              setRows((prev) => prev.map((p) => (p.name === row.name ? { ...p, on: !p.on } : p)));
            }}
          />
        ))}
    </PickerPanel>
  );
}

const ROWS: Row[] = [
  { name: 'alpha', on: true, description: 'The first one' },
  { name: 'beta', on: false },
  {
    name: 'gamma',
    on: false,
    disabled: true,
    note: 'Unavailable in this chat (public model)',
  },
  { name: 'delta', on: true },
];

describe('the picker shell', () => {
  it('is one combobox over one multi-select listbox of options', () => {
    render(<Harness initial={ROWS} />);
    const field = screen.getByRole('combobox', { name: 'Search extensions' });
    expect(field).toHaveAttribute('placeholder', 'Search extensions…');
    const list = screen.getByRole('listbox', { name: 'Extensions' });
    expect(list).toHaveAttribute('aria-multiselectable', 'true');
    const options = screen.getAllByRole('option');
    expect(options.map((o) => o.getAttribute('aria-selected'))).toEqual([
      'true',
      'false',
      'false',
      'true',
    ]);
  });

  it('draws each switch as the picture of its row: out of the accessibility tree and the tab order', () => {
    render(<Harness initial={ROWS} />);
    // The row is the control. A live switch inside it would be a second tab stop
    // and a second name for one thing.
    expect(screen.queryAllByRole('switch')).toHaveLength(0);
    const pictures = document.querySelectorAll('.br-switch');
    expect(pictures).toHaveLength(ROWS.length);
    pictures.forEach((picture) => {
      expect(picture).toHaveAttribute('aria-hidden', 'true');
      expect(picture).toHaveAttribute('tabindex', '-1');
    });
  });

  it('moves the highlight with the arrow keys and toggles the highlighted row on Enter', async () => {
    const onToggled = vi.fn();
    render(<Harness initial={ROWS} onToggled={onToggled} />);
    const field = screen.getByRole('combobox');
    field.focus();

    // The first row is highlighted on open; ArrowDown goes to the next ENABLED row,
    // because a refused row is never a stop.
    fireEvent.keyDown(field, { key: 'ArrowDown' });
    await waitFor(() =>
      expect(field.getAttribute('aria-activedescendant')).toBe(
        screen.getByRole('option', { name: /beta/ }).id
      )
    );
    fireEvent.keyDown(field, { key: 'ArrowDown' });
    await waitFor(() =>
      expect(field.getAttribute('aria-activedescendant')).toBe(
        screen.getByRole('option', { name: /delta/ }).id
      )
    );
    fireEvent.keyDown(field, { key: 'Enter' });
    expect(onToggled).toHaveBeenCalledTimes(1);
    expect(onToggled).toHaveBeenCalledWith('delta');
    expect(field).toHaveFocus();
    expect(screen.getByRole('option', { name: /delta/ })).toHaveAttribute('aria-selected', 'false');
  });

  it('keeps a refusal visible as a second line, and the row out of reach', () => {
    const onToggled = vi.fn();
    render(<Harness initial={ROWS} onToggled={onToggled} />);
    const refused = screen.getByRole('option', { name: /gamma/ });
    expect(refused).toHaveAttribute('aria-disabled', 'true');
    expect(refused).toHaveTextContent('Unavailable in this chat (public model)');
    fireEvent.click(refused);
    expect(onToggled).not.toHaveBeenCalled();
  });

  it('describes a row for a screen reader without renaming it or using title', () => {
    render(<Harness initial={ROWS} />);
    const alpha = screen.getByRole('option', { name: 'alpha' });
    expect(alpha).toHaveAccessibleDescription('The first one');
    expect(alpha).not.toHaveAttribute('title');
    expect(alpha.querySelector('[title]')).toBeNull();
  });

  it('puts the scope word and its InfoTip, then the bulk action, in the Tab order after the field', async () => {
    const user = userEvent.setup();
    render(<Harness initial={ROWS} />);
    expect(screen.getByText(TOOLS_COPY.scope.thisChat)).toBeInTheDocument();

    screen.getByRole('combobox').focus();
    await user.tab();
    const help = screen.getByRole('button', { name: `About ${TOOLS_COPY.scope.about}` });
    expect(help).toHaveFocus();
    expect(help).toHaveAccessibleDescription('Changes apply to this chat only.');

    await user.tab();
    expect(screen.getByRole('button', { name: 'Disable all (2)' })).toHaveFocus();
  });

  /**
   * `Command` answers Enter at its root for whichever row is highlighted, and the
   * scope line is inside that root. Enter on the bulk button used to toggle the
   * highlighted row and cancel the button's own activation.
   */
  it('gives Enter on the bulk action to the action, not to the highlighted row', async () => {
    const user = userEvent.setup();
    const onBulk = vi.fn();
    const onToggled = vi.fn();
    render(<Harness initial={ROWS} onBulk={onBulk} onToggled={onToggled} />);

    screen.getByRole('button', { name: 'Disable all (2)' }).focus();
    await user.keyboard('{Enter}');
    expect(onBulk).toHaveBeenCalledTimes(1);
    expect(onToggled).not.toHaveBeenCalled();
  });

  it('gives Enter on the InfoTip to the InfoTip, not to the highlighted row', async () => {
    const user = userEvent.setup();
    const onToggled = vi.fn();
    render(<Harness initial={ROWS} onToggled={onToggled} />);

    const help = screen.getByRole('button', { name: `About ${TOOLS_COPY.scope.about}` });
    help.focus();
    await user.keyboard('{Enter}');
    expect(onToggled).not.toHaveBeenCalled();
    expect(screen.getAllByRole('option', { selected: true })).toHaveLength(2);
  });

  it('shows the row’s description when the pointer rests on its badge, not only on its name', async () => {
    const user = userEvent.setup();
    render(
      <PickerPanel
        listLabel="Extensions"
        searchPlaceholder="Search extensions…"
        query=""
        onQueryChange={() => {}}
        scopeLabel="This chat"
        scopeHelp="Help."
        emptyText="None"
      >
        <PickerRow
          name="developer"
          checked
          onToggle={() => {}}
          badge={<BuiltInBadge />}
          description="Part of Biorouter. It can be turned off but not deleted."
        />
      </PickerPanel>
    );
    // Inside a row the badge is plain (the row is the control), so the sentence
    // it carries elsewhere rides on the row.
    await user.hover(screen.getByText('Built-in'));
    expect(await screen.findByRole('tooltip')).toHaveTextContent(
      'Part of Biorouter. It can be turned off but not deleted.'
    );
  });

  it('dims a refused row’s name and switch and leaves its reason at full strength', () => {
    render(<Harness initial={ROWS} />);
    const refused = screen.getByRole('option', { name: /gamma/ });
    // The rule itself is authored CSS (pickers.css) and measured in Chromium
    // (pickersGeometry.browser.test.tsx); here, that the hooks it keys on exist.
    expect(refused).toHaveAttribute('aria-disabled', 'true');
    expect(refused.querySelector('.br-picker-row__title')).not.toBeNull();
    expect(refused.querySelector('.br-picker-row__switch')).not.toBeNull();
    expect(refused.querySelector('.br-picker-row__note')).toHaveTextContent(
      'Unavailable in this chat (public model)'
    );
  });

  it('says so when a search matches nothing', () => {
    render(<Harness initial={ROWS} />);
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'zzz' } });
    expect(screen.queryAllByRole('option')).toHaveLength(0);
    expect(screen.getByText('No extensions found')).toBeInTheDocument();
  });

  /**
   * Principle 9 / spec 2.6: a switch the person moves slides; one that changes by
   * itself (a list that loaded, another window) snaps. The picture inside a row is
   * never clicked, so the row has to go THROUGH it, or every toggle reads as "not
   * the person's" and snaps. Both halves are pinned: the load snaps, the click slides.
   */
  it('snaps the switch of a row that changed by itself', () => {
    const { rerender } = render(
      <PickerPanel
        listLabel="Extensions"
        searchPlaceholder="Search extensions…"
        query=""
        onQueryChange={() => {}}
        scopeLabel="This chat"
        scopeHelp="Help."
        emptyText="None"
      >
        <PickerRow name="alpha" checked={false} onToggle={() => {}} />
      </PickerPanel>
    );
    const picture = () => document.querySelector('.br-switch') as HTMLElement;

    // Changed by itself: it loaded.
    rerender(
      <PickerPanel
        listLabel="Extensions"
        searchPlaceholder="Search extensions…"
        query=""
        onQueryChange={() => {}}
        scopeLabel="This chat"
        scopeHelp="Help."
        emptyText="None"
      >
        <PickerRow name="alpha" checked onToggle={() => {}} />
      </PickerPanel>
    );
    expect(picture()).toHaveAttribute('data-motion-still');
  });

  it('routes a row click through its switch, so the person’s change is not mistaken for a load', async () => {
    function One() {
      const [on, setOn] = React.useState(false);
      return (
        <PickerPanel
          listLabel="Extensions"
          searchPlaceholder="Search extensions…"
          query=""
          onQueryChange={() => {}}
          scopeLabel="This chat"
          scopeHelp="Help."
          emptyText="None"
        >
          <PickerRow name="alpha" checked={on} onToggle={() => setOn((value) => !value)} />
        </PickerPanel>
      );
    }
    render(<One />);
    const picture = document.querySelector('.br-switch') as HTMLElement;
    fireEvent.click(screen.getByRole('option', { name: 'alpha' }));
    expect(screen.getByRole('option', { name: 'alpha' })).toHaveAttribute('aria-selected', 'true');
    expect(picture).toHaveAttribute('data-state', 'checked');
    expect(picture).not.toHaveAttribute('data-motion-still');
  });
});
