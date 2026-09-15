import { useEffect, useMemo, useRef } from 'react';
import { analyzeDelimitedColumns, isMissingCell } from './artifactUtils';

/**
 * A written CSV/TSV as a data table on the panel's paper.
 *
 * - Columns never stretch: every data cell shrinks to its content and a
 *   trailing `.br-paper-fill` cell takes the slack, so the rules still span the
 *   760px column while a three-column file stays packed on the left.
 * - A quiet mono row index hangs in the left margin, the way the code gutter
 *   does, so the first data column sits on the same edge as a report's prose.
 * - Every row is one line: numbers right-align in tabular figures, a sentence
 *   column is clipped with its full value as the title, and a missing value
 *   (`NA`, `null`, …) recedes.
 *
 * All of it is authored CSS in main.css ("Delimited table"), keyed on the
 * classes and data attributes below.
 */
export default function DelimitedTable({ rows, maxRows }: { rows: string[][]; maxRows: number }) {
  const { header, shown, hidden, columns } = useMemo(() => {
    const [head = [], ...body] = rows;
    const visible = body.slice(0, maxRows);
    return {
      header: head,
      shown: visible,
      hidden: body.length - visible.length,
      // Typed from the rows that are SHOWN, so a hint never describes a cell the
      // reader cannot see.
      columns: analyzeDelimitedColumns(head, visible),
    };
  }, [rows, maxRows]);

  // The frame's left edge is the column edge — unless the table is too wide to
  // show whole from there, in which case it yields just enough to bring the last
  // column into view, and only a table wider than the whole panel scrolls. That
  // needs the table's real width, which CSS cannot read, so it is published as a
  // custom property the frame's padding resolves against (main.css). Without a
  // ResizeObserver (jsdom) the table simply sits on the column edge.
  const frameRef = useRef<HTMLDivElement>(null);
  const tableRef = useRef<HTMLTableElement>(null);
  useEffect(() => {
    const frame = frameRef.current;
    const table = tableRef.current;
    if (!frame || !table || typeof ResizeObserver === 'undefined') return;
    const sync = () => frame.style.setProperty('--paper-table-width', `${table.offsetWidth}px`);
    sync();
    const observer = new ResizeObserver(sync);
    observer.observe(table);
    return () => observer.disconnect();
  }, [rows]);

  if (rows.length === 0) {
    return <div className="br-preview-measure br-paper-empty">This file has no rows.</div>;
  }

  return (
    // Its own scroller, so the header can stick: a sticky cell sticks to its
    // nearest scrolling ancestor, and the frame's left padding puts the table on
    // the column edge while letting a wide table run past the column's right.
    <div className="br-paper-table-scroll" data-preview-scroller="">
      <div ref={frameRef} className="br-paper-table-frame">
        <table ref={tableRef} className="br-paper-table" data-preview-intrinsic="">
          <thead>
            <tr>
              <th className="br-paper-rownum" aria-hidden="true" />
              {header.map((cell, index) => (
                <th key={index} scope="col" data-numeric={columns[index]?.numeric || undefined}>
                  {cell}
                </th>
              ))}
              {/* The filler also carries the overflow hint through the opaque
                  sticky header, which would otherwise paint over the scroller's
                  own hint and notch it at this row (main.css). */}
              <th className="br-paper-fill" aria-hidden="true">
                <span className="br-paper-fill-hint" />
              </th>
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
        {hidden > 0 && (
          <p className="br-paper-table-note">
            {hidden.toLocaleString()} more row{hidden === 1 ? '' : 's'} not shown. Open the raw view
            for the full file.
          </p>
        )}
      </div>
    </div>
  );
}
