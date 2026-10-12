import { useId, type ComponentType, type ReactNode } from 'react';
import type { LucideProps } from 'lucide-react';

type EmptyStateIcon = ComponentType<LucideProps>;

interface EmptyStateProps {
  icon: EmptyStateIcon;
  title: string;
  description: string;
  actions?: ReactNode;
  className?: string;
  compact?: boolean;
}

export function EmptyState({
  icon: Icon,
  title,
  description,
  actions,
  className = '',
  compact = false,
}: EmptyStateProps) {
  const titleId = useId();
  const descriptionId = useId();

  return (
    <section
      aria-labelledby={titleId}
      aria-describedby={descriptionId}
      className={`mx-auto flex w-full max-w-lg flex-col items-center justify-center px-4 text-center ${
        compact ? 'py-12' : 'min-h-[clamp(17rem,48vh,25rem)] py-12'
      } ${className}`.trim()}
    >
      {/* The mark's tile is a subtle fill, not an outline (R-27); the transparent border keeps
          its size and gives forced colours an edge. `.br-empty-tile` (main.css) rests it on the
          control ground, so it still shows on a recessed ground and inside a filled box. */}
      <div className="br-empty-tile mb-5 flex h-12 w-12 items-center justify-center rounded-container border border-transparent text-text-muted">
        <Icon className="h-6 w-6" aria-hidden="true" />
      </div>
      <h2 id={titleId} className="text-subheading text-text-default">
        {title}
      </h2>
      <p id={descriptionId} className="mt-2 max-w-sm text-body text-text-muted">
        {description}
      </p>
      {actions && (
        <div className="mt-5 flex flex-wrap items-center justify-center gap-2">{actions}</div>
      )}
    </section>
  );
}
