import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SteerUnavailableNote } from './SteerUnavailableNote';
import { STEER_UNAVAILABLE_SHORT, STEER_UNAVAILABLE_WHY } from './steerUnavailableCopy';

let browserSurface = true;
vi.mock('../../utils/surface', () => ({ isBrowserSurface: () => browserSurface }));

/**
 * SD-8's note, as the owner's rule shapes it (2026-11-13): one line saying what cannot happen and
 * which button does work, and the why behind the info icon beside it, where a screen reader still
 * hears it as the icon's description.
 */
describe('SteerUnavailableNote', () => {
  beforeEach(() => {
    browserSurface = true;
  });
  afterEach(() => {
    browserSurface = true;
  });

  it('renders nothing on the desktop', () => {
    browserSurface = false;
    const { container } = render(<SteerUnavailableNote />);
    expect(container).toBeEmptyDOMElement();
  });

  it('shows one line naming Stop & send, with the reason behind the info icon', () => {
    render(<SteerUnavailableNote />);
    const note = screen.getByTestId('steer-unavailable-note');
    expect(note.firstChild?.textContent).toBe(STEER_UNAVAILABLE_SHORT);
    expect(note.firstChild?.textContent).toMatch(/Stop & send/);
    const about = screen.getByRole('button', { name: 'About steering in a browser' });
    expect(about).toHaveAccessibleDescription(STEER_UNAVAILABLE_WHY);
    expect(STEER_UNAVAILABLE_WHY).toMatch(/biorouter serve/);
    expect(STEER_UNAVAILABLE_WHY).toMatch(/desktop app/);
  });
});
