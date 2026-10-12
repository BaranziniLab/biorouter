import { useId, type HTMLAttributes } from 'react';
import { Button } from '../../ui/button';
import { ExternalLink } from '../../icons/app-icons';
import { aboutCopy } from './copy';

const BUG_REPORT_URL =
  'https://github.com/BaranziniLab/biorouter/issues/new?template=bug_report.md';
const FEATURE_REQUEST_URL =
  'https://github.com/BaranziniLab/biorouter/issues/new?template=feature_request.md';

/**
 * The two feedback exits. Both leave the app for GitHub in the default browser, so each label is
 * followed by the same 14px external-link glyph the other exits in Settings carry (the Switch
 * models quick start guide, View info's official URL), and both are described "Opens in your
 * browser" for a person who cannot see the glyph. The glyph is decoration (`aria-hidden`); the
 * description is one hidden element both buttons point at, so it is announced after the name and
 * stays out of it (the accessible name of each is still its visible label). The group takes the
 * id, name and description `SettingRow` hands its one control, so it is spread onto the wrapper.
 *
 * The glyph is `size-3.5`, not `h-3.5 w-3.5`: the button primitive sizes every svg that does not
 * carry a `size-` class to 16px with a selector that outranks a plain height/width utility.
 */
export default function FeedbackLinks(group: HTMLAttributes<HTMLDivElement>) {
  const descriptionId = useId();
  const links = [
    { url: BUG_REPORT_URL, label: aboutCopy.reportBug },
    { url: FEATURE_REQUEST_URL, label: aboutCopy.requestFeature },
  ];
  return (
    // `group` carries the id, name and description `SettingRow` gives its one control.
    <div {...group} role="group" className="flex items-center gap-1">
      {links.map(({ url, label }) => (
        <Button
          key={url}
          variant="ghost"
          size="sm"
          // 4px between label and glyph, as the other exits space theirs (the small button's own
          // 6px would leave the glyph hanging off the label).
          className="gap-1"
          aria-describedby={descriptionId}
          onClick={() => window.open(url, '_blank')}
        >
          {label}
          <ExternalLink aria-hidden="true" className="size-3.5 text-text-muted" />
        </Button>
      ))}
      <span id={descriptionId} hidden>
        {aboutCopy.opensInBrowser}
      </span>
    </div>
  );
}
