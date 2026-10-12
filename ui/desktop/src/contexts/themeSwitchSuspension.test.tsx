import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ThemeProvider } from './ThemeContext';
import ThemeSelector from '../components/BioRouterSidebar/ThemeSelector';
import ThemeFamilySelector from '../components/BioRouterSidebar/ThemeFamilySelector';
import { THEME_SWITCHING_CLASS } from '../styles/motion';

/**
 * A theme change is a jump: the page snaps its grounds, so every control that eases its colour
 * would otherwise walk from the old theme to the new one over 95 to 125ms, a dark button on a
 * white page (vision V4-15). `ThemeContext` puts `biorouter-theme-switching` on <html> for the
 * frame the mode or the palette changes in, and only then.
 *
 * jsdom runs no style or transitions, so what these pin is the wiring: the class is on <html> in
 * the same task as the flip, it comes off two animation frames later, and a mount or the effect
 * that follows a click (the value is already applied) starts nothing. The rule itself and the
 * helper are measured in a real browser by `styles/themeSwitch.browser.test.ts`.
 */
const root = () => document.documentElement;
const switching = () => root().classList.contains(THEME_SWITCHING_CLASS);

let frames: ((time: number) => void)[] = [];
const runFrame = () => {
  const due = frames;
  frames = [];
  due.forEach((callback) => callback(0));
};

function renderProvider() {
  return render(
    <ThemeProvider>
      <ThemeSelector />
      <ThemeFamilySelector />
    </ThemeProvider>
  );
}

describe('ThemeContext stills transitions for a theme change', () => {
  beforeEach(() => {
    localStorage.clear();
    localStorage.setItem('use_system_theme', 'false');
    localStorage.setItem('theme', 'light');
    localStorage.setItem('theme_family', 'parchment');
    root().classList.remove('dark', 'light', THEME_SWITCHING_CLASS);
    root().removeAttribute('data-theme');
    frames = [];
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => {
      frames.push(callback);
      return frames.length;
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    root().classList.remove(THEME_SWITCHING_CLASS);
  });

  it('suspends for the frame a mode click lands in, and only until the second frame', () => {
    renderProvider();
    act(() => {
      runFrame();
      runFrame();
    });
    expect(switching()).toBe(false);

    fireEvent.click(screen.getByTestId('dark-mode-button'));
    expect(root().classList.contains('dark')).toBe(true);
    expect(switching()).toBe(true);

    act(() => runFrame());
    expect(switching()).toBe(true);
    act(() => runFrame());
    expect(switching()).toBe(false);
  });

  it('suspends for a palette click', () => {
    renderProvider();
    act(() => {
      runFrame();
      runFrame();
    });
    expect(switching()).toBe(false);

    fireEvent.click(screen.getByTestId('theme-family-alma-mater-button'));
    expect(root().getAttribute('data-theme')).toBe('alma-mater');
    expect(switching()).toBe(true);
    act(() => {
      runFrame();
      runFrame();
    });
    expect(switching()).toBe(false);
  });

  it('starts nothing when a light page mounts, with or without a light class on <html> (FR-R4-1)', () => {
    // The boot splash fades out in the frame the app mounts. A mount that stills every transition
    // on <html> stills that fade too, so a light page must not look like a change from nothing.
    // index.html stamps `light` itself now; a bare <html> is the same page and must read the same.
    for (const stamped of [[], ['light']]) {
      root().className = '';
      stamped.forEach((name) => root().classList.add(name));
      root().setAttribute('data-theme', 'parchment');
      frames = [];
      const view = renderProvider();
      expect(switching(), `classes ${JSON.stringify(stamped)}`).toBe(false);
      expect(frames, `classes ${JSON.stringify(stamped)}`).toHaveLength(0);
      expect(root().classList.contains('light')).toBe(true);
      expect(root().classList.contains('dark')).toBe(false);
      view.unmount();
    }
  });

  it('starts nothing when a dark page mounts over the dark class index.html stamped', () => {
    localStorage.setItem('theme', 'dark');
    root().classList.add('dark');
    root().setAttribute('data-theme', 'parchment');
    renderProvider();
    expect(switching()).toBe(false);
    expect(frames).toHaveLength(0);
  });

  it('still suspends when a mount really lands on the other mode than the page was stamped with', () => {
    // The page says dark and the saved choice is light (a choice made in another window, say):
    // the mode does change on mount, and a change is a jump.
    root().classList.add('dark');
    root().setAttribute('data-theme', 'parchment');
    renderProvider();
    expect(root().classList.contains('light')).toBe(true);
    expect(switching()).toBe(true);
    act(() => {
      runFrame();
      runFrame();
    });
    expect(switching()).toBe(false);

    // And the other way: a bare <html> is a light page, so a dark choice over it is a change.
    root().className = '';
    localStorage.setItem('theme', 'dark');
    cleanup();
    renderProvider();
    expect(root().classList.contains('dark')).toBe(true);
    expect(switching()).toBe(true);
  });

  it('starts nothing when the mode or the palette is already what the page shows', () => {
    root().classList.add('light');
    root().setAttribute('data-theme', 'parchment');
    renderProvider();
    expect(switching()).toBe(false);
    expect(frames).toHaveLength(0);

    // Choosing what is already chosen applies the same class and attribute again.
    fireEvent.click(screen.getByTestId('light-mode-button'));
    fireEvent.click(screen.getByTestId('theme-family-parchment-button'));
    expect(switching()).toBe(false);
    expect(frames).toHaveLength(0);
  });
});
