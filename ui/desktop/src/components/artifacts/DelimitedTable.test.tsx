import { act, cleanup, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import DelimitedTable from './DelimitedTable';

/**
 * The two things a CSV's frame needs from the browser's layout, which CSS cannot
 * read and jsdom does not compute (so this fakes the numbers; main.css's rules
 * on `data-yield` and `data-overflow` are pinned in styles/artifactPaper.test.ts,
 * and the harness shows them drawn):
 *
 * - `data-yield`: the frame leaves the column edge only when that brings the
 *   whole table into view. A table wider than the panel keeps the edge, which
 *   is what the vision pass measured outdented (V2-09, 10px against 28px).
 * - `data-overflow`: the scroller says whether more lies to its right, which is
 *   what turns the right-edge fade on and off.
 */

const rows = [
  ['gene', 'p'],
  ['MYC', '0.01'],
  ['KRAS', '0.02'],
];

let observers: Array<{ callback: () => void; targets: Element[] }> = [];

beforeEach(() => {
  observers = [];
  vi.stubGlobal(
    'ResizeObserver',
    class {
      entry: { callback: () => void; targets: Element[] };
      constructor(callback: () => void) {
        this.entry = { callback, targets: [] };
        observers.push(this.entry);
      }
      observe(target: Element) {
        this.entry.targets.push(target);
      }
      disconnect() {
        this.entry.targets = [];
      }
      unobserve() {}
    }
  );
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

type Layout = { table: number; viewport: number; scrollWidth?: number; scrollLeft?: number };

function mount(layout: Layout) {
  const original = Object.getOwnPropertyDescriptors(HTMLElement.prototype);
  const define = (name: string, get: (element: HTMLElement) => number) =>
    Object.defineProperty(HTMLElement.prototype, name, {
      configurable: true,
      get() {
        return get(this as HTMLElement);
      },
    });
  define('offsetWidth', (element) => (element.tagName === 'TABLE' ? layout.table : 0));
  define('clientWidth', (element) =>
    element.classList.contains('br-paper-table-scroll') ? layout.viewport : 0
  );
  define('scrollWidth', (element) =>
    element.classList.contains('br-paper-table-scroll') ? (layout.scrollWidth ?? layout.table) : 0
  );
  define('scrollLeft', (element) =>
    element.classList.contains('br-paper-table-scroll') ? (layout.scrollLeft ?? 0) : 0
  );
  // The gutter and the lead: what the stylesheet would resolve for a 600px panel.
  const real = window.getComputedStyle.bind(window);
  vi.spyOn(window, 'getComputedStyle').mockImplementation((element, pseudo) => {
    const style = real(element, pseudo);
    if (!(element as HTMLElement).classList?.contains('br-paper-table-frame')) return style;
    return new Proxy(style, {
      get(target, property) {
        if (property === 'paddingRight') return '27px';
        if (property === 'getPropertyValue') {
          return (name: string) =>
            name === '--paper-lead' ? '10px' : target.getPropertyValue(name);
        }
        const value = Reflect.get(target, property);
        return typeof value === 'function' ? value.bind(target) : value;
      },
    });
  });
  const view = render(<DelimitedTable rows={rows} maxRows={500} />);
  const restore = () => {
    for (const name of ['offsetWidth', 'clientWidth', 'scrollWidth', 'scrollLeft']) {
      const descriptor = original[name];
      if (descriptor) Object.defineProperty(HTMLElement.prototype, name, descriptor);
      else delete (HTMLElement.prototype as unknown as Record<string, unknown>)[name];
    }
    vi.restoreAllMocks();
  };
  const frame = view.container.querySelector<HTMLElement>('.br-paper-table-frame')!;
  const scroller = view.container.querySelector<HTMLElement>('.br-paper-table-scroll')!;
  return { frame, scroller, restore, view };
}

describe('DelimitedTable layout flags', () => {
  it('keeps the column edge for a table too wide to show whole from any edge', () => {
    // 1300px of table in a 599px panel: yielding to the 10px lead would not bring
    // it into view, only outdent it.
    const { frame, scroller, restore } = mount({ table: 1300, viewport: 599, scrollWidth: 1337 });
    try {
      expect(frame).not.toHaveAttribute('data-yield');
      expect(frame.style.getPropertyValue('--paper-table-width')).toBe('1300px');
      // …and says there is more to the right, so the fade is drawn.
      expect(scroller).toHaveAttribute('data-overflow', 'true');
    } finally {
      restore();
    }
  });

  it('yields from the column edge when that is what shows the whole table', () => {
    // 10 + 555 + 27 = 592, which fits in 599; from the column edge it would not.
    const { frame, scroller, restore } = mount({ table: 555, viewport: 599 });
    try {
      expect(frame).toHaveAttribute('data-yield');
      expect(scroller).not.toHaveAttribute('data-overflow');
    } finally {
      restore();
    }
  });

  it('turns the fade off at the scroll end and back on when the reader scrolls away', () => {
    const layout: Layout = { table: 1300, viewport: 599, scrollWidth: 1337, scrollLeft: 0 };
    const { scroller, restore } = mount(layout);
    try {
      expect(scroller).toHaveAttribute('data-overflow', 'true');
      layout.scrollLeft = 738;
      act(() => {
        scroller.dispatchEvent(new Event('scroll'));
      });
      expect(scroller).not.toHaveAttribute('data-overflow');
      layout.scrollLeft = 300;
      act(() => {
        scroller.dispatchEvent(new Event('scroll'));
      });
      expect(scroller).toHaveAttribute('data-overflow', 'true');
    } finally {
      restore();
    }
  });

  it('re-measures when the panel is resized', () => {
    const layout: Layout = { table: 555, viewport: 599 };
    const { frame, scroller, restore } = mount(layout);
    try {
      expect(frame).toHaveAttribute('data-yield');
      // The panel narrows: the table no longer fits even from the lead.
      layout.viewport = 480;
      layout.scrollWidth = 600;
      act(() => {
        for (const observer of observers) observer.callback();
      });
      expect(frame).not.toHaveAttribute('data-yield');
      expect(scroller).toHaveAttribute('data-overflow', 'true');
      // Both the table and the scroller are watched.
      expect(observers.flatMap((observer) => observer.targets)).toContain(scroller);
    } finally {
      restore();
    }
  });

  it('draws no fade of its own on a filler cell: the stylesheet does, from data-overflow', () => {
    const { view, restore } = mount({ table: 1300, viewport: 599, scrollWidth: 1337 });
    try {
      const fillers = view.container.querySelectorAll('.br-paper-fill');
      expect(fillers).toHaveLength(rows.length);
      fillers.forEach((cell) => expect(cell).toBeEmptyDOMElement());
    } finally {
      restore();
    }
  });
});
