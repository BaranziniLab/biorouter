import { homeFolder, tildePath } from '../../utils/folderPath';

/**
 * " Recorded in <path>." — the one place a notice names the record, shared
 * with Settings → Privacy's strip. The path says where the record lives, so it
 * is set in the sentence's own face from `~`, as every folder that is only read
 * is (design.md D-42; `utils/folderPath.ts`), and allowed to break anywhere: a
 * home directory is long enough to overrun a narrow composer.
 */
export function RecordedIn({ path }: { path: string | null }) {
  if (!path) return null;
  return (
    <>
      {' '}
      Recorded in{' '}
      <span dir="ltr" data-path={path} className="[overflow-wrap:anywhere]">
        {tildePath(path, homeFolder())}
      </span>
      .
    </>
  );
}
