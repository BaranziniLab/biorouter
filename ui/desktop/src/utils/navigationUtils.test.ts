import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { NavigateFunction } from 'react-router-dom';
import { renderHook } from '@testing-library/react';
import {
  createNavigationHandler,
  navigateToRoute,
  navigateWithViewTransition,
  ROUTE_SURFACE_SELECTOR,
  useRouteFade,
} from './navigationUtils';

describe('navigation helpers', () => {
  const navigate = vi.fn() as unknown as NavigateFunction;

  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    Reflect.deleteProperty(document, 'startViewTransition');
  });

  it('updates the route synchronously without starting a document snapshot transition', () => {
    const startViewTransition = vi.fn();
    Object.defineProperty(document, 'startViewTransition', {
      configurable: true,
      value: startViewTransition,
    });

    navigateToRoute(navigate, '/settings', { section: 'models' });

    expect(startViewTransition).not.toHaveBeenCalled();
    expect(navigate).toHaveBeenCalledWith('/settings', {
      replace: undefined,
      state: { section: 'models' },
    });
  });

  it('preserves every rapid navigation intent in order', () => {
    navigateToRoute(navigate, '/settings');
    navigateToRoute(navigate, '/workflows');
    navigateToRoute(navigate, '/knowledge');

    expect(navigate).toHaveBeenNthCalledWith(1, '/settings', {
      replace: undefined,
      state: undefined,
    });
    expect(navigate).toHaveBeenNthCalledWith(2, '/workflows', {
      replace: undefined,
      state: undefined,
    });
    expect(navigate).toHaveBeenNthCalledWith(3, '/knowledge', {
      replace: undefined,
      state: undefined,
    });
  });

  it('keeps the shared view mapping and replace option intact', () => {
    const setView = createNavigationHandler(navigate);

    setView('chat', { disableAnimation: true });
    navigateToRoute(navigate, '/sessions', undefined, { replace: true });

    expect(navigate).toHaveBeenNthCalledWith(1, '/', {
      replace: undefined,
      state: { disableAnimation: true },
    });
    expect(navigate).toHaveBeenNthCalledWith(2, '/sessions', {
      replace: true,
      state: undefined,
    });
  });
});

describe('the old name', () => {
  it('is the same function until its last importer moves', () => {
    expect(navigateWithViewTransition).toBe(navigateToRoute);
  });
});

/**
 * Spec 3.16, Routes: the main column fades in once when the path changes, and
 * nowhere else. jsdom has no WAAPI, so `animate` is a stub on the surface.
 */
describe('useRouteFade', () => {
  let surface: HTMLElement;
  let animate: ReturnType<typeof vi.fn>;
  const originalMatchMedia = window.matchMedia;

  beforeEach(() => {
    surface = document.createElement('main');
    surface.className = 'route-container biorouter-route-surface';
    animate = vi.fn();
    Object.assign(surface, { animate });
    document.body.appendChild(surface);
  });

  afterEach(() => {
    surface.remove();
    window.matchMedia = originalMatchMedia;
    document.body.classList.remove('biorouter-window-resizing');
  });

  it('targets the shell surface', () => {
    expect(document.querySelector(ROUTE_SURFACE_SELECTOR)).toBe(surface);
  });

  it('does nothing on the first mount: a reload or a new window appears at rest', () => {
    renderHook(({ path }) => useRouteFade(path), { initialProps: { path: '/settings' } });
    expect(animate).not.toHaveBeenCalled();
  });

  it('fades the column in once, opacity only, when the path changes', () => {
    const { rerender } = renderHook(({ path }) => useRouteFade(path), {
      initialProps: { path: '/settings' },
    });
    rerender({ path: '/workflows' });

    expect(animate).toHaveBeenCalledTimes(1);
    expect(animate).toHaveBeenCalledWith([{ opacity: 0 }, { opacity: 1 }], {
      duration: 125,
      easing: 'cubic-bezier(0.24, 1, 0.4, 1)',
    });

    // The same path again (a query-string change, a re-render) fades nothing.
    rerender({ path: '/workflows' });
    expect(animate).toHaveBeenCalledTimes(1);
  });

  it('stays still under reduced motion and while the window is being resized', () => {
    window.matchMedia = ((query: string) =>
      ({
        matches: query === '(prefers-reduced-motion: reduce)',
      }) as MediaQueryList) as typeof window.matchMedia;
    const { rerender } = renderHook(({ path }) => useRouteFade(path), {
      initialProps: { path: '/settings' },
    });
    rerender({ path: '/skills' });
    expect(animate).not.toHaveBeenCalled();

    window.matchMedia = originalMatchMedia;
    document.body.classList.add('biorouter-window-resizing');
    rerender({ path: '/knowledge' });
    expect(animate).not.toHaveBeenCalled();
  });
});
