import * as React from 'react';
import { Badge } from './badge';
import { InfoTip } from './info-tip';
import { uiCopy } from './copy';

/** A badge inside one of these rows sits in something that is itself the control. */
const ROW_CONTROL_SELECTOR =
  '[role="menuitem"], [role="menuitemcheckbox"], [role="menuitemradio"], [role="option"]';

/**
 * The one "Built-in" badge (Skills, Extensions, Workflows, Scheduler, Knowledge, the composer's
 * menus). It is the `Badge` primitive in sentence case (spec 2.1 and 2.6: no caps on a badge),
 * and its explanation is an `InfoTip asChild`: the badge itself becomes the focusable trigger,
 * so the help opens on hover, on Tab and on click and is linked by `aria-describedby`, never a
 * `title=` (AppTooltipLayer would turn a title into the badge's name).
 *
 * Inside a menu or listbox row the row is the control, and a focusable trigger in it would eat
 * the click that picks the row (spec 2.6, InfoTip "Never"), so there the badge is plain.
 *
 * `title` keeps its name for the callers that pass their own help (a recreated built-in); line
 * breaks in it fold into one line, since the help is now a sentence, not a native tooltip.
 */
export default function BuiltInBadge({ title = uiCopy.builtInHelp }: { title?: string }) {
  const [inRow, setInRow] = React.useState(false);
  const help = title.replace(/\s*\n\s*/g, ' ').trim();
  const notice = React.useCallback((node: HTMLElement | null) => {
    if (node?.parentElement?.closest(ROW_CONTROL_SELECTOR)) setInRow(true);
  }, []);

  if (inRow) {
    return (
      <Badge ref={notice} tone="neutral">
        {uiCopy.builtIn}
      </Badge>
    );
  }

  return (
    <InfoTip ref={notice} label={uiCopy.builtIn} help={help} asChild>
      <Badge tone="neutral">{uiCopy.builtIn}</Badge>
    </InfoTip>
  );
}
