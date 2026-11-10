import * as React from 'react';

/**
 * The classes every facet chip wears, verbatim from the Knowledge graph's facet strip
 * (`knowledge/graph/GraphFacetStrip.tsx`), where they already generate. `.br-facet`
 * in `main.css` (WS-KNOWLEDGE) carries the resting fill and the engaged pair keyed on
 * `data-engaged='true'`; nothing here adds a rule.
 */
export const FACET_CHIP_CLASS_NAME =
  'br-facet biorouter-focus-surface inline-flex h-control-md flex-none items-center gap-2 rounded-element px-2.5 text-label transition-[color,background-color,border-color]';

export interface FacetChipProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  /**
   * Whether the facet holds a value: `true` paints the engaged fill. `false` writes
   * `data-engaged="false"` (the strip's markup); leaving it out writes no attribute,
   * for a chip that is never engaged (the graph's Legend).
   */
  engaged?: boolean;
}

/**
 * FacetChip: a filter chip that opens a menu or a popover (the Knowledge graph's
 * facet strip, the decision history's Source, Outcome and Filters). One control for
 * the job, so the two surfaces cannot drift into two copies of a long class string.
 *
 * Use it as the child of `DropdownMenuTrigger asChild` or `PopoverTrigger asChild`;
 * it forwards its ref and spreads the trigger's props. `type` defaults to `button`.
 */
export const FacetChip = React.forwardRef<HTMLButtonElement, FacetChipProps>(
  ({ engaged, className, type = 'button', ...props }, ref) => (
    <button
      ref={ref}
      type={type}
      className={className ? `${FACET_CHIP_CLASS_NAME} ${className}` : FACET_CHIP_CLASS_NAME}
      data-engaged={engaged === undefined ? undefined : engaged ? 'true' : 'false'}
      {...props}
    />
  )
);
FacetChip.displayName = 'FacetChip';
