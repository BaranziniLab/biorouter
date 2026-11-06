import { useLayoutEffect, useRef } from 'react';
import { folderSegments, homeFolder, tildePath } from '../utils/folderPath';
import { PathSegments } from './ui/path-segments';
import './terminal-dock.css';

/**
 * The written folder in two pieces: what may shrink (everything before the last name) and what
 * stays (the last separator and the last name). A path with one name, a root (`/`, `C:\`) or
 * the home folder (`~`) is all name.
 */
function splitCwd(written: string): { lead: string; name: string } {
  const segments = folderSegments(written);
  const last = segments[segments.length - 1];
  if (!last || last.separator || segments.length < 2) return { lead: '', name: written };
  return {
    lead: segments
      .slice(0, -2)
      .map((segment) => segment.text)
      .join(''),
    name: segments[segments.length - 2].text + last.text,
  };
}

/**
 * Size the leading part to the glyphs it keeps, so its ellipsis touches the folder's name.
 *
 * `text-overflow: ellipsis` hides whole glyphs and draws the ellipsis right after the last one
 * that fits, so the rest of the box the strip gave the leading part stayed empty: up to a glyph
 * of space between the ellipsis and the name (`~/bi… /home`, 2 to 8px measured in Chromium at
 * 12px). This finds the width that glyph run and the ellipsis take and caps the box there; the
 * space left over goes to the pane tabs, which take whatever the strip has left. A leading part
 * that is not cut, or of which no glyph fits, is left to the stylesheet.
 */
function fitCwdLead(lead: HTMLElement): void {
  lead.style.maxWidth = '';
  const range = document.createRange();
  // jsdom lays nothing out and has no range geometry.
  if (typeof range.getBoundingClientRect !== 'function') return;
  const box = lead.getBoundingClientRect();
  const texts = document.createTreeWalker(lead, NodeFilter.SHOW_TEXT);
  const rights: number[] = [];
  for (let node = texts.nextNode(); node; node = texts.nextNode()) {
    const length = node.textContent?.length ?? 0;
    for (let i = 0; i < length; i++) {
      range.setStart(node, i);
      range.setEnd(node, i + 1);
      rights.push(range.getBoundingClientRect().right - box.left);
    }
  }
  if (rights.length === 0 || Math.max(...rights) <= box.width + 0.01) return;
  // The ellipsis in the leading part's own face and size.
  const probe = document.createElement('span');
  probe.textContent = '…';
  probe.style.cssText = 'position: absolute; visibility: hidden; white-space: nowrap';
  lead.appendChild(probe);
  const ellipsis = probe.getBoundingClientRect().width;
  probe.remove();
  const kept = Math.max(0, ...rights.filter((right) => right + ellipsis <= box.width + 0.01));
  // 1/16px over, so layout's own rounding cannot drop the last glyph that fits.
  if (kept > 0) lead.style.maxWidth = `${kept + ellipsis + 0.0625}px`;
}

/**
 * The terminal's working folder, beside the pane tabs, written as the app writes a folder
 * (`tildePath(path, homeFolder())`: `~` for the home folder, absolute where the daemon is not
 * this machine's). The folder's own name always stays in view: when the strip is short the
 * leading part shrinks behind an ellipsis (`~/Desk…/BioRouter`, the ellipsis against the name,
 * `fitCwdLead`), and the name is cut only when it alone is wider than the space
 * (`terminal-dock.css`). The whole path is still the element's text, for a screen reader and a
 * selection. Each name is its own `<bdi>`, so a right-to-left name cannot reorder the separators.
 */
export function TerminalCwd({ cwd }: { cwd?: string }) {
  const leadRef = useRef<HTMLSpanElement>(null);
  const written = cwd ? tildePath(cwd, homeFolder()) : '';
  // Refit when the folder changes and whenever the strip (the space on offer) or the name (its
  // face or size) changes size. Neither changes when the leading part's box does.
  useLayoutEffect(() => {
    const lead = leadRef.current;
    if (!lead) return;
    const fit = () => fitCwdLead(lead);
    fit();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(fit);
    const strip = lead.parentElement?.parentElement;
    if (strip) observer.observe(strip);
    if (lead.nextElementSibling) observer.observe(lead.nextElementSibling);
    return () => observer.disconnect();
  }, [written]);

  if (!cwd) {
    return (
      <span className="br-terminal-cwd text-supporting text-text-muted">
        <span className="br-terminal-cwd__name">Chat terminal</span>
      </span>
    );
  }
  const { lead, name } = splitCwd(written);
  return (
    <span dir="ltr" data-path={cwd} className="br-terminal-cwd text-supporting text-text-muted">
      {lead ? (
        <span ref={leadRef} className="br-terminal-cwd__lead">
          <PathSegments path={lead} breaks={false} />
        </span>
      ) : null}
      <span className="br-terminal-cwd__name">
        <PathSegments path={name} breaks={false} />
      </span>
    </span>
  );
}
