import React from 'react';
import { folderSegments, type FolderSegment } from '../../utils/folderPath';

/**
 * A written path drawn in pieces: each folder name in its own `<bdi>`, the separators beside it
 * outside the isolate, so inside a `dir="ltr"` parent a right-to-left name cannot reorder the
 * separators around it. The parent decides how the path is written (`tildePath(path,
 * homeFolder())`, or the exact absolute path where the folder is the argument of a decision) and
 * its own face.
 *
 * - `breaks` (the default) groups each name with the separator after it (a leading separator
 *   with the first name, so a root never ends a line alone) in a `.br-path-piece` span, which the
 *   parent's stylesheet draws as one atomic box no wider than the line (`.br-footer-card__path`
 *   in pickers.css, `.br-tool-preview-path` in tool-call.css). A line then breaks only between two
 *   pieces, which is after a separator. Drawn as plain text, a path also broke after a hyphen or
 *   at a space inside a name ("biorouter-" / "workflows-and-skills"), where a break at the
 *   separator just before it would have fitted, and took no break at all after a backslash
 *   before a letter or after "/" before a digit; a box around the name alone let a separator
 *   start the next line, because an atomic box may break on either side (both measured in
 *   Chromium, footerCard.browser.test.tsx). Only a name wider than the line breaks inside its
 *   piece, at its own spaces and hyphens first, then by the parent's `overflow-wrap: anywhere`.
 * - `breaks={false}` draws the names and separators flat, for a single line that is cut rather
 *   than wrapped (the terminal dock's folder).
 * - Neither the pieces nor the `<bdi>`s add a character: a selection copies the path as written.
 *   The parent's stylesheet keeps every space of a name (`white-space: pre-wrap` on the
 *   `<bdi>`), because a name begins its piece, an atomic box, and a space a name starts with would
 *   otherwise be dropped at the start of that box's line, drawn and copied ("/Users/me/ .ssh" as
 *   "/Users/me/.ssh").
 */
export function PathSegments({ path, breaks = true }: { path: string; breaks?: boolean }) {
  const segments = folderSegments(path);
  if (!breaks) return <>{segments.map(drawSegment)}</>;
  return (
    <>
      {pathPieces(segments).map((piece, index) => (
        <span key={index} className="br-path-piece">
          {piece.map(drawSegment)}
        </span>
      ))}
    </>
  );
}

function drawSegment(segment: FolderSegment, index: number) {
  return segment.separator ? (
    <React.Fragment key={index}>{segment.text}</React.Fragment>
  ) : (
    <bdi key={index}>{segment.text}</bdi>
  );
}

/**
 * The segments grouped into what a line may not break inside: a name with the separators after
 * it, and any separator before the first name with that name ("/", "Volumes", "/" is one piece of
 * "/Volumes/Lab"; "~", "/" of "~/Desktop"). A path of separators alone is one piece.
 */
function pathPieces(segments: FolderSegment[]): FolderSegment[][] {
  const pieces: FolderSegment[][] = [];
  let current: FolderSegment[] = [];
  for (const segment of segments) {
    if (!segment.separator && current.some((held) => !held.separator)) {
      pieces.push(current);
      current = [];
    }
    current.push(segment);
  }
  if (current.length > 0) pieces.push(current);
  return pieces;
}
