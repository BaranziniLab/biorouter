import { useEffect, useMemo, useRef, useState } from 'react';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { Input } from '../../ui/input';
import { Textarea } from '../../ui/textarea';
import { knowledgeCopy } from '../copy';

interface Props {
  onStage: (text: string, title: string, urls: string[]) => void;
  onCancel: () => void;
}

/**
 * Paste-a-chunk-of-prose staging (ui-spec §4.4).
 *
 * Two corrections over the hand-rolled box this replaces:
 *
 * - **The title field is the `Input` primitive**, and the bespoke
 *   `focus-within:border-border-strong` on the container is gone. Focus is
 *   global (D-15) — the field deepens its own fill and firms its own edge — and
 *   a container that also reacted meant two focus treatments stacked on one
 *   gesture.
 * - **Detected URLs are `Badge variant="chip"` toggles**, not hand-rolled
 *   `py-0.5` spans. `asChild` so the chip IS the button and takes the global
 *   focus fill; without it an opaque `bg-background-medium` span would sit on
 *   top of its own focus surface and a keyboard user would see nothing.
 */
export function PasteTextBox({ onStage, onCancel }: Props) {
  const [text, setText] = useState('');
  const [title, setTitle] = useState('');
  const textRef = useRef<HTMLTextAreaElement>(null);

  /**
   * The box is *summoned*, so the caret belongs in it.
   *
   * `preventScroll` is load-bearing rather than defensive: the panel scrolls
   * this box clear of its pinned footer in a layout effect, and a focus that
   * did its own scrolling would fight that — landing the box back under the
   * footer, which is the defect the scroll exists to fix.
   */
  useEffect(() => {
    textRef.current?.focus({ preventScroll: true });
  }, []);

  const detectedUrls = useMemo(() => {
    // Redeclare regex inside useMemo to avoid sharing lastIndex state across calls
    const urlRe = /https?:\/\/[^\s<>"')]+[^\s<>"').,;:!?]/g;
    const set = new Set<string>();
    let m: RegExpExecArray | null;
    while ((m = urlRe.exec(text)) !== null) set.add(m[0]);
    return Array.from(set);
  }, [text]);

  const [includeUrls, setIncludeUrls] = useState<Record<string, boolean>>({});
  const urlsToFetch = detectedUrls.filter((u) => includeUrls[u] !== false);

  function toggleUrl(u: string) {
    setIncludeUrls((prev) => ({ ...prev, [u]: prev[u] !== false ? false : true }));
  }

  return (
    <div className="flex flex-col gap-2 rounded-container border border-border-subtle bg-background-default p-3">
      <Input
        type="text"
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        placeholder={knowledgeCopy.ingest.pasteTitlePlaceholder}
        aria-label="Title for the pasted source"
      />
      {/* The `Textarea` primitive, the one multi-line field: Input's edge and
          radius, resizing vertically only. Five rows because this is a paste
          target. It keeps the focus surface this box has always carried
          (`focusSurface.test.ts` pins it for the Knowledge controls that write
          their own background). */}
      <Textarea
        ref={textRef}
        rows={5}
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder={knowledgeCopy.ingest.pastePlaceholder}
        aria-label="Text to stage"
        className="biorouter-focus-surface"
      />

      {detectedUrls.length > 0 && (
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-supporting text-text-muted">Will fetch:</span>
          {detectedUrls.map((u) => {
            const on = includeUrls[u] !== false;
            return (
              <Badge
                key={u}
                variant="chip"
                asChild
                className={
                  on
                    ? 'tint-selected tint-interactive text-text-default'
                    : 'tint-interactive text-text-muted line-through'
                }
              >
                <button type="button" aria-pressed={on} onClick={() => toggleUrl(u)}>
                  {u.length > 36 ? u.substring(0, 33) + '…' : u}
                </button>
              </Badge>
            );
          })}
        </div>
      )}

      <div className="flex items-center justify-between gap-2">
        <span className="text-supporting tabular-nums text-text-muted">{text.length} chars</span>
        <div className="flex gap-2">
          <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
            Cancel
          </Button>
          <Button
            type="button"
            size="sm"
            disabled={!text.trim()}
            onClick={() => onStage(text.trim(), title.trim() || 'Pasted knowledge', urlsToFetch)}
          >
            Stage
          </Button>
        </div>
      </div>
    </div>
  );
}
