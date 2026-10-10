import { X } from './icons/app-icons';
import type { QuoteReference } from '../utils/quotedText';
import './composer/composer.css';

/**
 * A quotation attached to a message: where it came from, then the words, under
 * a 2px rule with no fill (the same quiet recipe as a markdown blockquote,
 * spec 2.1), so a quote inside the composer card or a message bubble is never
 * a box inside a box. The source's locator (a path or an address) is in the
 * tooltip, not on the line.
 */
export function QuotedTextChip({
  quote,
  onRemove,
}: {
  quote: QuoteReference;
  onRemove?: () => void;
}) {
  return (
    <span data-testid="quoted-text-chip" className="br-quote-chip">
      <span className="br-quote-chip__body">
        <span className="br-quote-chip__source text-chip" title={quote.sourceLocator}>
          {quote.label}
        </span>
        <span className="br-quote-chip__text text-secondary" dir="auto">
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
