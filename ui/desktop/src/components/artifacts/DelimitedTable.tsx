import { useLayoutEffect, useMemo, useRef } from 'react';
import { analyzeDelimitedColumns, isMissingCell } from './artifactUtils';

/**
 * A written CSV/TSV as a data table on the panel's paper.
 *
 * - It wears the app's one table (`.br-md-table`, Crew's recipe, which chat,
 *   documents and notebook DataFrames share; spec 3.8), with a header that
 *   sticks while the rows scroll under it.
 * - Columns never stretch: every data cell shrinks to its content and a
 *   trailing `.br-paper-fill` cell takes the slack, so the frame still spans the
 *   760px column while a three-column file stays packed on the left.
 * - A quiet mono row index is the first column.
 * - Every row is one line: numbers right-align in tabular figures, a sentence
 *   column is clipped with its full value as the title, and a missing value
 *   (`NA`, `null`, …) recedes.
 * - Past the cap, the bar above says "500 of 620 rows" and its InfoTip says
 *   why; the table itself carries no note.
 * - The frame's left edge is the column edge the bar's name shares, whatever the
 *   table's width; the one exception is a table that fits only by sitting nearer
 *   the panel's edge (`data-yield`). A table wider than the panel scrolls, and
 *   fades out at the right edge while more lies that way (`data-overflow`).
 *
 * All of it is authored CSS in main.css ("Delimited table"), keyed on the
 * classes and data attributes below.
 */
export default function DelimitedTable({ rows, maxRows }: { rows: string[][]; maxRows: number }) {
  const { header, shown, columns } = useMemo(() => {
    const [head = [], ...body] = rows;
    const visible = body.slice(0, maxRows);
    return {
      header: head,
      shown: visible,
      // Typed from the rows that are SHOWN, so a hint never describes a cell the
      // reader cannot see.
      columns: analyzeDelimitedColumns(head, visible),
    };
  }, [rows, maxRows]);

  // The frame's left edge is the column edge, the edge the bar's name and the
  // prose share (checklist 11). It yields from it only when yielding is what
  // brings the whole table into view, and only as far as that takes; a table
  // wider than the panel could not be shown whole from any edge, so it keeps
  // the column edge and scrolls. Both answers need the table's real width, which
  // CSS cannot read, so it is measured here and published for the frame's padding
  // to resolve against (`--paper-table-width`, `data-yield`; main.css). The same
  // pass tells the scroller whether more lies to its right (`data-overflow`),
  // which is what draws the fade there. In a layout effect, so the first paint
  // is already the right one. Without a ResizeObserver (jsdom) the table simply
  // sits on the column edge with no fade.
  const scrollerRef = useRef<HTMLDivElement>(null);
  const frameRef = useRef<HTMLDivElement>(null);
  const tableRef = useRef<HTMLTableElement>(null);
  useLayoutEffect(() => {
    const scroller = scrollerRef.current;
    const frame = frameRef.current;
    const table = tableRef.current;
    if (!scroller || !frame || !table || typeof ResizeObserver === 'undefined') return;
    const syncOverflow = () => {
      const more = scroller.scrollWidth - scroller.clientWidth - scroller.scrollLeft > 1;
      if (more) scroller.setAttribute('data-overflow', 'true');
      else scroller.removeAttribute('data-overflow');
    };
    const sync = () => {
      const tableWidth = table.offsetWidth;
      frame.style.setProperty('--paper-table-width', `${tableWidth}px`);
      const style = getComputedStyle(frame);
      const lead = Number.parseFloat(style.getPropertyValue('--paper-lead')) || 0;
      const gutter = Number.parseFloat(style.paddingRight) || 0;
      frame.toggleAttribute('data-yield', lead + tableWidth + gutter <= scroller.clientWidth);
      syncOverflow();
    };
    sync();
    const observer = new ResizeObserver(sync);
    observer.observe(table);
    observer.observe(scroller);
    scroller.addEventListener('scroll', syncOverflow, { passive: true });
    return () => {
      observer.disconnect();
      scroller.removeEventListener('scroll', syncOverflow);
    };
  }, [rows]);

  if (rows.length === 0) {
    return <div className="br-preview-measure br-paper-empty">This file has no rows.</div>;
  }

  return (
    // Its own scroller, so the header can stick: a sticky cell sticks to its
    // nearest scrolling ancestor, and the frame's left padding puts the table on
    // the column edge while letting a wide table run past the column's right.
    <div ref={scrollerRef} className="br-paper-table-scroll" data-preview-scroller="">
      <div ref={frameRef} className="br-paper-table-frame">
        <table ref={tableRef} className="br-md-table br-paper-table" data-preview-intrinsic="">
          <thead>
            <tr>
              <th className="br-paper-rownum" aria-hidden="true" />
              {header.map((cell, index) => (
                <th key={index} scope="col" data-numeric={columns[index]?.numeric || undefined}>
                  {cell}
                </th>
              ))}
              {/* The filler of every row is also where the right-edge fade is
                  drawn: it sticks to the scroller's right edge while the table
                  runs past it, so each row fades on its own ground (main.css). */}
              <th className="br-paper-fill" aria-hidden="true" />
            </tr>
          </thead>
          <tbody>
            {shown.map((row, rowIndex) => (
              <tr key={rowIndex}>
                <td className="br-paper-rownum" aria-hidden="true">
                  {rowIndex + 1}
                </td>
                {header.map((_, cellIndex) => {
                  const value = row[cellIndex] ?? '';
                  const prose = columns[cellIndex]?.prose;
                  return (
                    <td
                      key={cellIndex}
                      data-numeric={columns[cellIndex]?.numeric || undefined}
                      data-prose={prose || undefined}
                      data-missing={isMissingCell(value) || undefined}
                    >
                      {/* A sentence column is clipped to one line, never wrapped:
                          a wrapped cell off to the right sets the height of the
                          whole row, so rows you CAN see went uneven for text you
                          could not. The full value is the title, and Raw. */}
                      {prose ? (
                        <span className="br-paper-cell-clip" title={value}>
                          {value}
                        </span>
                      ) : (
                        value
                      )}
                    </td>
                  );
                })}
                <td className="br-paper-fill" aria-hidden="true" />
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
