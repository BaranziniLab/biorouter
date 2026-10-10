import * as React from 'react';

import { cn } from '../../utils';
import { InfoTip } from '../ui/info-tip';
import './usage.css';

/**
 * StatGrid: the one stat style, shared by Home's usage card and the Settings
 * usage report (spec 3.9). A `<dl>` of flat tiles: a 12px muted label over a
 * 24px regular figure with tabular numbers. No tile boxes, no icons and no
 * detail lines: an explanation is the label's InfoTip, and the only text that
 * stays visible under a figure is a short `note` that changes what the figure
 * means ("Known conservative subtotal").
 *
 * Three columns, two under a 480px container (a container query on the
 * frame, so the grid answers to its own width rather than the window's).
 *
 * Motion: when `animateKey` changes (a range switch), each figure remounts
 * with a 125ms fade, and nothing counts up. The first render is at rest, so a
 * card that mounts because of navigation never fades in.
 */

export interface StatItem {
  /** Stable identity; also the figure's key, so a changed `animateKey` remounts it. */
  key: string;
  label: string;
  /** The figure. */
  value: React.ReactNode;
  /**
   * The exact figure as a hover tooltip, for a compact or truncatable value
   * ("87.9B" → "87,912,345,678"). Never help text: that is `help`.
   */
  title?: string;
  /** A muted 14px tail on the figure's line ("/30"). */
  suffix?: React.ReactNode;
  /** A 12px muted line under the figure, for a qualifier that must stay visible. */
  note?: React.ReactNode;
  /** Plain-text help behind an InfoTip beside the label (two sentences at most). */
  help?: string;
  testId?: string;
}

export interface StatGridProps {
  items: readonly StatItem[];
  /** Changes when the figures change for a reason worth showing (a range switch). */
  animateKey?: string;
  className?: string;
  'aria-label'?: string;
  'data-testid'?: string;
}

export function StatGrid({
  items,
  animateKey,
  className,
  'aria-label': ariaLabel,
  'data-testid': testId,
}: StatGridProps) {
  // The key the grid mounted with: figures fade only once it has moved on.
  const [initialKey] = React.useState(animateKey);
  const animate = animateKey !== initialKey;

  return (
    <div className={cn('br-stat-grid-frame', className)}>
      <dl className="br-stat-grid" aria-label={ariaLabel} data-testid={testId}>
        {items.map((item) => (
          <div key={item.key} className="br-stat" data-testid={item.testId}>
            <dt className="br-stat-label text-supporting text-text-muted">
              <span className="br-stat-label-text">{item.label}</span>
              {item.help ? <InfoTip label={item.label} help={item.help} /> : null}
            </dt>
            <dd
              key={`${animateKey ?? ''}:${item.key}`}
              className="br-stat-value text-title text-text-default"
              data-animate={animate ? '' : undefined}
            >
              <span className="br-stat-figure" title={item.title}>
                {item.value}
              </span>
              {item.suffix ? (
                <span className="br-stat-suffix text-body text-text-muted">{item.suffix}</span>
              ) : null}
            </dd>
            {item.note ? (
              <dd className="br-stat-note text-supporting text-text-muted">{item.note}</dd>
            ) : null}
          </div>
        ))}
      </dl>
    </div>
  );
}
