import { useCallback, useEffect, useRef, useState } from 'react';
import { DUR, EASE_OUT, prefersReducedMotion } from '../../styles/motion';

/**
 * The text that was just sent, drawn where it was typed while it lifts away
 * (`.br-composer-send-ghost`, composer.css). Coordinates are the field's box in
 * the composer card's padding box, where the ghost is absolutely positioned,
 * measured from the card's BOTTOM edge: the card is anchored there and settles
 * back to one line as the ghost lifts (`useFieldGrowth`), so a ghost hung from
 * the top would sink with the card's top edge.
 */
export interface SendGhost {
  /** A fresh value per send, so a second send restarts the motion. */
  id: number;
  text: string;
  bottom: number;
  left: number;
  width: number;
  height: number;
  /** The field's scroll position, so a long draft lifts from where it was read. */
  scrollTop: number;
}

/**
 * The composer's send motion (owner, 2026-11-13: "when the text is getting sent").
 *
 * Two parts, both transform and opacity only (`styles/motion.ts`), both skipped
 * under reduced motion:
 *  - the sent text lifts 8px and fades from the field (a ghost copy, because the
 *    field itself is cleared in the same tick), over `--dur-med-min`;
 *  - Send's arrow leaves upward and comes back from below, the press answered.
 *
 * The field empties at once and the card closes to one line at once, so the
 * motion never holds up the next thing typed.
 */
export function useSendMotion() {
  const [ghost, setGhost] = useState<SendGhost | null>(null);
  const glyphRef = useRef<HTMLSpanElement | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const nextId = useRef(0);

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    []
  );

  const clearGhost = useCallback(() => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    setGhost(null);
  }, []);

  const play = useCallback((field: HTMLTextAreaElement | null, text: string) => {
    if (prefersReducedMotion()) return;

    const glyph = glyphRef.current;
    if (glyph && typeof glyph.animate === 'function') {
      glyph.animate(
        [
          { transform: 'translateY(0)', opacity: 1 },
          { transform: 'translateY(-10px)', opacity: 0, offset: 0.45 },
          { transform: 'translateY(8px)', opacity: 0, offset: 0.46 },
          { transform: 'translateY(0)', opacity: 1 },
        ],
        { duration: DUR.medMin, easing: EASE_OUT }
      );
    }

    const card = field?.closest<HTMLElement>('.br-composer-card');
    const shown = text.trim();
    if (!field || !card || !shown) return;
    const fieldBox = field.getBoundingClientRect();
    const cardBox = card.getBoundingClientRect();
    // The card's border is outside its padding box, the ghost's containing
    // block; its bottom border is as wide as its top one.
    const bottom = cardBox.bottom - fieldBox.bottom - card.clientTop;
    const left = fieldBox.left - cardBox.left - card.clientLeft;
    nextId.current += 1;
    setGhost({
      id: nextId.current,
      text,
      bottom,
      left,
      width: fieldBox.width,
      height: fieldBox.height,
      scrollTop: field.scrollTop,
    });
    if (timer.current) clearTimeout(timer.current);
    // `animationend` clears it; this is the fallback when no animation ran.
    timer.current = setTimeout(() => {
      timer.current = null;
      setGhost(null);
    }, DUR.medMin + 120);
  }, []);

  return { ghost, clearGhost, play, glyphRef };
}
