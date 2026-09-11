/**
 * " Recorded in <path>." — the one place a notice names the record, shared
 * with Settings → Privacy's strip. The path is where to look, so it is set in
 * the mono face paths take everywhere in Settings, and allowed to break
 * anywhere: a home directory is long enough to overrun a narrow composer.
 */
export function RecordedIn({ path }: { path: string | null }) {
  if (!path) return null;
  return (
    <>
      {' '}
      Recorded in <span className="font-mono [overflow-wrap:anywhere]">{path}</span>.
    </>
  );
}
