import * as React from 'react';

import { cn } from '../../utils';
import { InfoTip, useInfoTipId } from './info-tip';

/**
 * SettingSection and SettingRow: the one settings recipe (spec 2.6), shared by every Settings
 * workstream and by Crew-like rows elsewhere.
 *
 * ```tsx
 * <SettingSection title="General" help="…" action={<Button variant="ghost" shape="round" …/>}>
 *   <SettingRow label="Prevent sleep while running" help="The screen can still lock."
 *               status="Restart to apply" controlId="prevent-sleep">
 *     <Switch id="prevent-sleep" checked={…} onCheckedChange={…} />
 *   </SettingRow>
 * </SettingSection>
 * ```
 *
 * Row (`.biorouter-settings-row`, 40px minimum, padding 10px 12px, gap 12px):
 *   - left: a real `<label htmlFor={controlId}>` in `text-label`, truncated, then the InfoTip as
 *     the label's SIBLING (never inside it: a click on the glyph would toggle the control);
 *   - an optional `status` line under the label in `text-supporting` muted, for transient state
 *     only ("Restart to apply", "Up to date", "Managed by your organization");
 *   - an optional `value` in `text-secondary` muted before the control;
 *   - ONE control at the trailing edge, which the row gives `id`, `aria-labelledby` (the label)
 *     and `aria-describedby` (the help) unless the control already names itself.
 * A click on the row's empty space toggles a switch or checkbox control, the way a click on the
 * label does; clicks on the InfoTip and on any other interactive element are left alone.
 *
 * Section: a `text-caps` muted label with an optional InfoTip and an optional action at its end,
 * then the rows in `.biorouter-settings-list`. No paragraph under the header: the explanation is
 * the InfoTip.
 */

export const settingLabelId = (controlId: string) => `${controlId}-label`;
export const settingStatusId = (controlId: string) => `${controlId}-status`;

const NO_ROW_TOGGLE_SELECTOR =
  'button, a[href], input, select, textarea, label, [role="switch"], [role="checkbox"], [role="radio"], [role="combobox"], [role="menuitem"], .br-info-tip, .br-info-tip-target, [data-row-no-toggle]';

export interface SettingRowProps {
  /** The row's name; it is also the control's accessible name. */
  label: React.ReactNode;
  /** Plain-text explanation behind an InfoTip (two sentences at most). */
  help?: string;
  /** A muted current value shown before the control (`mono` for versions and IDs only). */
  value?: React.ReactNode;
  /** Draw `value` in the mono face: versions and IDs only. */
  valueMono?: boolean;
  /** A transient state line under the label ("Restart to apply"). */
  status?: React.ReactNode;
  /** The control's id. Generated (and given to the control) when omitted. */
  controlId?: string;
  /** The one control, at the trailing edge. */
  children?: React.ReactNode;
  disabled?: boolean;
  className?: string;
  'data-testid'?: string;
}

function mergeIds(...ids: Array<string | undefined | null | false>): string | undefined {
  const joined = ids.filter(Boolean).join(' ').trim();
  return joined.length > 0 ? joined : undefined;
}

export function SettingRow({
  label,
  help,
  value,
  valueMono = false,
  status,
  controlId: controlIdProp,
  children,
  disabled,
  className,
  'data-testid': testId,
}: SettingRowProps) {
  const generatedId = React.useId();
  const controlId = controlIdProp ?? `setting-${generatedId.replace(/[^a-zA-Z0-9_-]/g, '')}`;
  const helpId = useInfoTipId();
  const labelId = settingLabelId(controlId);
  const statusId = settingStatusId(controlId);
  const labelText = typeof label === 'string' ? label : undefined;

  // One element is enhanced; anything else (a fragment, several nodes) renders as given.
  const control = React.isValidElement<Record<string, unknown>>(children) ? children : null;
  const controlProps = (control ? control.props : {}) as Record<string, unknown>;
  const describedBy = mergeIds(
    controlProps['aria-describedby'] as string | undefined,
    help ? helpId : undefined,
    status ? statusId : undefined
  );
  const enhancedControl = control
    ? React.cloneElement(control, {
        id: (controlProps.id as string | undefined) ?? controlId,
        'aria-labelledby':
          controlProps['aria-labelledby'] ??
          (controlProps['aria-label'] === undefined ? labelId : undefined),
        'aria-describedby': describedBy,
      })
    : (children ?? null);

  const rowRef = React.useRef<HTMLDivElement>(null);
  const onRowClick = (event: React.MouseEvent<HTMLDivElement>) => {
    if (disabled) return;
    const target = event.target as Element | null;
    if (!target || target.closest(NO_ROW_TOGGLE_SELECTOR)) return;
    const row = rowRef.current;
    const toggle = row?.ownerDocument.getElementById(controlId);
    if (!row || !toggle || !row.contains(toggle)) return;
    const isToggle =
      toggle.getAttribute('role') === 'switch' ||
      toggle.getAttribute('role') === 'checkbox' ||
      (toggle instanceof HTMLInputElement && toggle.type === 'checkbox');
    if (!isToggle) return;
    if (toggle.hasAttribute('disabled') || toggle.getAttribute('aria-disabled') === 'true') return;
    toggle.click();
  };

  return (
    <div
      ref={rowRef}
      className={cn('biorouter-settings-row br-setting-row', className)}
      data-slot="setting-row"
      data-disabled={disabled ? 'true' : undefined}
      data-testid={testId}
      onClick={onRowClick}
    >
      <div className="br-setting-row-text">
        <div className="br-setting-row-head">
          <label
            id={labelId}
            htmlFor={controlId}
            className="br-setting-row-label text-label text-text-default"
          >
            {label}
          </label>
          {help ? <InfoTip label={labelText ?? 'this setting'} help={help} id={helpId} /> : null}
        </div>
        {status ? (
          <span id={statusId} className="br-setting-row-status text-supporting text-text-muted">
            {status}
          </span>
        ) : null}
      </div>
      {value !== undefined && value !== null ? (
        <span
          className={cn(
            'br-setting-row-value text-secondary text-text-muted',
            valueMono && 'font-mono'
          )}
        >
          {value}
        </span>
      ) : null}
      {enhancedControl ? <div className="br-setting-row-control">{enhancedControl}</div> : null}
    </div>
  );
}

export interface SettingSectionProps {
  /** The caps section label. */
  title: React.ReactNode;
  /** Plain-text explanation behind an InfoTip beside the label. */
  help?: string;
  /** One optional action at the header's end (a ghost round button, a link). */
  action?: React.ReactNode;
  /** The section's id; deep links scroll to it. The heading is `${id}-title`. */
  id?: string;
  children?: React.ReactNode;
  className?: string;
  'data-testid'?: string;
}

export function SettingSection({
  title,
  help,
  action,
  id,
  children,
  className,
  'data-testid': testId,
}: SettingSectionProps) {
  const generatedId = React.useId();
  const titleId = `${id ?? `section-${generatedId.replace(/[^a-zA-Z0-9_-]/g, '')}`}-title`;
  const titleText = typeof title === 'string' ? title : undefined;
  return (
    <section
      id={id}
      aria-labelledby={titleId}
      className={cn('biorouter-settings-section br-setting-section', className)}
      data-slot="setting-section"
      data-testid={testId}
    >
      <div className="biorouter-settings-section-header br-setting-section-header">
        <h2 id={titleId} className="br-setting-section-title text-caps text-text-muted">
          {title}
        </h2>
        {help ? <InfoTip label={titleText ?? 'this section'} help={help} /> : null}
        {action ? <div className="br-setting-section-action">{action}</div> : null}
      </div>
      <div className="biorouter-settings-list">{children}</div>
    </section>
  );
}
