import './probabilityBar.css';

/**
 * A thin probability bar (4px), with the threshold's mark when there is one.
 * Drawn, never announced: the figure written beside it is what is read. Shared
 * by the chat summary's Decisions rows and the decision history view.
 */
export function ProbabilityBar({
  p,
  threshold,
  className,
}: {
  p: number | null;
  threshold?: number | null;
  className?: string;
}) {
  const percent = (value: number) => `${Math.round(Math.min(1, Math.max(0, value)) * 1000) / 10}%`;
  return (
    <span className={className ? `br-pbar ${className}` : 'br-pbar'} aria-hidden="true">
      {p !== null ? <span className="br-pbar__fill" style={{ width: percent(p) }} /> : null}
      {threshold !== null && threshold !== undefined ? (
        <span className="br-pbar__mark" style={{ left: percent(threshold) }} />
      ) : null}
    </span>
  );
}

export default ProbabilityBar;
