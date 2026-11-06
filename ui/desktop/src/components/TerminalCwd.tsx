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
 * The terminal's working folder, beside the pane tabs, written as the app writes a folder
 * (`tildePath(path, homeFolder())`: `~` for the home folder, absolute where the daemon is not
 * this machine's). The folder's own name always stays in view: when the strip is short the
 * leading part shrinks behind an ellipsis (`~/Desk…/BioRouter`), and the name is cut only when it
 * alone is wider than the space (`terminal-dock.css`). The whole path is still the element's
 * text, for a screen reader and a selection. Each name is its own `<bdi>`, so a right-to-left
 * name cannot reorder the separators.
 */
export function TerminalCwd({ cwd }: { cwd?: string }) {
  if (!cwd) {
    return (
      <span className="br-terminal-cwd text-supporting text-text-muted">
        <span className="br-terminal-cwd__name">Chat terminal</span>
      </span>
    );
  }
  const { lead, name } = splitCwd(tildePath(cwd, homeFolder()));
  return (
    <span dir="ltr" data-path={cwd} className="br-terminal-cwd text-supporting text-text-muted">
      {lead ? (
        <span className="br-terminal-cwd__lead">
          <PathSegments path={lead} breaks={false} />
        </span>
      ) : null}
      <span className="br-terminal-cwd__name">
        <PathSegments path={name} breaks={false} />
      </span>
    </span>
  );
}
