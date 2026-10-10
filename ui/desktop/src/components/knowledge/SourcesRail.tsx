import { IngestPanel } from './IngestPanel/IngestPanel';

interface Props {
  className?: string;
}

/**
 * The Sources rail: the left column of the Knowledge workspace.
 *
 * A flush column, not a card (spec 3.12 "Flatten"). The columns divide by an inset hairline
 * (`.br-knowledge-sources` in `main.css`, Crew's column edge), so there is no box inside a box and
 * no 40px "SOURCES" strip: at the narrow step the tab already says "Sources", and at the wide
 * step the dropzone says what the column is for.
 *
 * ⚠ **The tier control no longer lives here.** It moved into the band, where it is visible at
 * every width; below the 860px step this rail hides behind the default Graph tab, so the tier
 * used to be off screen there (issue #56 DR-18 asks that the tier sit beside the base it acts on,
 * and the band is that place).
 *
 * ⚠ **THE RAIL OWNS NO SCROLL CONTAINER** (R-06). The ingest panel's footer is a flex SIBLING of
 * its scroller, so it cannot paint over the body; a `sticky bottom-0` footer once occluded the
 * paste box and made "Paste text" read as a dead button.
 */
export function SourcesRail({ className = '' }: Props) {
  return (
    <section
      aria-label="Sources"
      className={`flex min-h-0 flex-col overflow-hidden bg-background-default ${className}`.trim()}
    >
      <IngestPanel />
    </section>
  );
}
