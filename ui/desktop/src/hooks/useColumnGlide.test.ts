import { renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DUR, EASE_OUT } from '../styles/motion';
import { captureScrollAnchor, GLIDE_CAPTURE_TTL_MS, useColumnGlide } from './useColumnGlide';

/**
 * jsdom lays nothing out and implements no WAAPI, so each test builds the split
 * box by hand: the grid items, the measured elements inside them with a left
 * edge the test controls, and an `animate` spy on the items the glide moves.
 */
type Fixture = ReturnType<typeof buildSplit>;

function rectAt(left: number) {
  return { left, top: 0, right: left + 760, bottom: 100, width: 760, height: 100 } as DOMRect;
}

function buildSplit(splitWidth = 1152) {
  const split = document.createElement('div');
  Object.defineProperty(split, 'clientWidth', { configurable: true, value: splitWidth });
  const transcript = document.createElement('div');
  transcript.setAttribute('data-preview-area', 'transcript');
  const column = document.createElement('div');
  column.className = 'biorouter-chat-column';
  transcript.appendChild(column);
  const bar = document.createElement('div');
  bar.setAttribute('data-preview-area', 'composer');
  const shell = document.createElement('div');
  shell.setAttribute('data-composer-shell', 'true');
  bar.appendChild(shell);
  split.append(transcript, bar);
  document.body.appendChild(split);

  const lefts = { column: 196, shell: 196 };
  column.getBoundingClientRect = () => rectAt(lefts.column);
  shell.getBoundingClientRect = () => rectAt(lefts.shell);
  const animations: Array<{
    element: Element;
    keyframes: Array<Record<string, string>>;
    options: unknown;
  }> = [];
  for (const element of [transcript, bar]) {
    (element as HTMLElement).animate = vi.fn(
      (keyframes: Array<Record<string, string>>, options: unknown) => {
        animations.push({ element, keyframes, options });
        return { cancel: vi.fn(), onfinish: null, oncancel: null } as unknown as Animation;
      }
    ) as unknown as HTMLElement['animate'];
  }
  return { split, transcript, bar, column, shell, lefts, animations };
}

function hookFor(fixture: Fixture) {
  const ref = { current: fixture.split };
  return renderHook(() => useColumnGlide(ref)).result;
}

let reduced = false;
beforeEach(() => {
  reduced = false;
  vi.stubGlobal(
    'matchMedia',
    vi.fn((query: string) => ({ matches: reduced && query.includes('reduce'), media: query }))
  );
  window.matchMedia = globalThis.matchMedia;
});
afterEach(() => {
  document.body.innerHTML = '';
  document.body.className = '';
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('useColumnGlide', () => {
  it('FLIPs the transcript cell and the composer bar from their captured place', () => {
    const fixture = buildSplit();
    const glide = hookFor(fixture);
    glide.current.capture();
    // The preview took its column: the column now starts at 28.
    fixture.lefts.column = 28;
    fixture.lefts.shell = 28;
    expect(glide.current.playCaptured({ duration: DUR.med })).toBe(true);
    expect(fixture.animations.map((a) => a.element)).toEqual([fixture.transcript, fixture.bar]);
    expect(fixture.animations[0].keyframes).toEqual([
      { transform: 'translateX(168px)' },
      { transform: 'translateX(0)' },
    ]);
    expect(fixture.animations[0].options).toEqual({ duration: DUR.med, easing: EASE_OUT });
  });

  it('glides by a known distance for the rail', () => {
    const fixture = buildSplit();
    const glide = hookFor(fixture);
    glide.current.glideBy(140, { duration: DUR.med });
    expect(fixture.animations.map((a) => a.element)).toEqual([fixture.transcript, fixture.bar]);
    expect(fixture.animations[0].keyframes[0]).toEqual({ transform: 'translateX(140px)' });
    glide.current.glideBy(-140, { duration: DUR.fastMax, composer: false });
    expect(fixture.animations).toHaveLength(3);
    expect(fixture.animations[2].options).toEqual({ duration: DUR.fastMax, easing: EASE_OUT });
  });

  it('does not glide while the window or the sidebar is being resized', () => {
    for (const name of ['biorouter-window-resizing', 'biorouter-sidebar-resizing']) {
      const fixture = buildSplit();
      const glide = hookFor(fixture);
      document.body.classList.add(name);
      glide.current.glideBy(140, { duration: DUR.med });
      glide.current.capture();
      fixture.lefts.column = 28;
      glide.current.playCaptured({ duration: DUR.med });
      expect(fixture.animations, name).toHaveLength(0);
      document.body.classList.remove(name);
    }
  });

  it('does not glide under reduced motion', () => {
    reduced = true;
    const fixture = buildSplit();
    const glide = hookFor(fixture);
    glide.current.glideBy(140, { duration: DUR.med });
    glide.current.capture();
    fixture.lefts.column = 28;
    glide.current.playCaptured({ duration: DUR.med });
    expect(fixture.animations).toHaveLength(0);
  });

  it('snaps a move under 8px', () => {
    const fixture = buildSplit();
    const glide = hookFor(fixture);
    glide.current.glideBy(7, { duration: DUR.med });
    glide.current.capture();
    fixture.lefts.column = 190;
    fixture.lefts.shell = 190;
    glide.current.playCaptured({ duration: DUR.med });
    expect(fixture.animations).toHaveLength(0);
  });

  it('skips when the window grew to seat the preview, or the pane itself changed width', () => {
    const fixture = buildSplit();
    const glide = hookFor(fixture);
    glide.current.capture({ windowGrowing: true });
    fixture.lefts.column = 28;
    expect(glide.current.playCaptured({ duration: DUR.med })).toBe(true);
    expect(fixture.animations).toHaveLength(0);

    glide.current.capture();
    Object.defineProperty(fixture.split, 'clientWidth', { configurable: true, value: 1300 });
    glide.current.playCaptured({ duration: DUR.med });
    expect(fixture.animations).toHaveLength(0);
  });

  it('forgets a capture nothing committed against in time', () => {
    vi.useFakeTimers();
    const fixture = buildSplit();
    const glide = hookFor(fixture);
    glide.current.capture();
    vi.advanceTimersByTime(GLIDE_CAPTURE_TTL_MS + 1);
    fixture.lefts.column = 28;
    expect(glide.current.playCaptured({ duration: DUR.med })).toBe(false);
    expect(fixture.animations).toHaveLength(0);
    // A capture is consumed once.
    expect(glide.current.playCaptured({ duration: DUR.med })).toBe(false);
  });

  it('leaves the composer to its own FLIP when it is already moving', () => {
    const fixture = buildSplit();
    (fixture.shell as HTMLElement).getAnimations = () =>
      [{ playState: 'running' }] as unknown as Animation[];
    const glide = hookFor(fixture);
    glide.current.glideBy(140, { duration: DUR.med });
    expect(fixture.animations.map((a) => a.element)).toEqual([fixture.transcript]);
  });
});

describe('captureScrollAnchor', () => {
  it('scrolls by however far the clicked card moved', () => {
    const viewport = document.createElement('div');
    const card = document.createElement('button');
    viewport.appendChild(card);
    document.body.appendChild(viewport);
    viewport.scrollTop = 400;
    let cardTop = 300;
    viewport.getBoundingClientRect = () => ({ top: 50 }) as DOMRect;
    card.getBoundingClientRect = () => ({ top: cardTop }) as DOMRect;
    const restore = captureScrollAnchor(viewport, card);
    expect(restore).not.toBeNull();
    // The narrower column reflowed: the card is 120px lower than it was.
    cardTop = 420;
    restore!();
    expect(viewport.scrollTop).toBe(520);
  });

  it('anchors nothing outside the transcript', () => {
    const viewport = document.createElement('div');
    const elsewhere = document.createElement('button');
    document.body.append(viewport, elsewhere);
    expect(captureScrollAnchor(viewport, elsewhere)).toBeNull();
    expect(captureScrollAnchor(viewport, null)).toBeNull();
    expect(captureScrollAnchor(null, elsewhere)).toBeNull();
  });
});
