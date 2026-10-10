import { X } from './icons/app-icons';
import type { QuoteReference } from '../utils/quotedText';
import './composer/composer.css';

export function QuotedTextChip({
  quote,
  onRemove,
}: {
  quote: QuoteReference;
  onRemove?: () => void;
}) {
  return (
    <span
      data-testid="quoted-text-chip"
      className="inline-flex max-w-full items-start gap-2 rounded-element border border-border-subtle bg-background-muted px-3 py-2 text-supporting text-text-default"
    >
      <span className="min-w-0">
        <span className="block text-chip" title={quote.sourceLocator}>
          {quote.label}
        </span>
        <span
          className="block max-h-24 overflow-auto whitespace-pre-wrap break-words font-normal"
          dir="auto"
        >
          {quote.value}
        </span>
      </span>
      {onRemove && (
        <button
          type="button"
          onClick={onRemove}
          aria-label={`Remove quote from ${quote.label}`}
          className="br-composer-chip-action"
        >
          <X className="size-3.5" aria-hidden />
        </button>
      )}
    </span>
  );
}
