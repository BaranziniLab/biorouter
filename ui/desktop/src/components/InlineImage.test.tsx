import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { InlineImage } from './InlineImage';
import { transcriptCopy } from './transcript/copy';

const copy = transcriptCopy.image;

describe('InlineImage', () => {
  const getTempImage = vi.fn();

  beforeEach(() => {
    getTempImage.mockReset();
    (window as unknown as { electron: unknown }).electron = { getTempImage };
  });

  afterEach(() => {
    delete (window as unknown as { electron?: unknown }).electron;
  });

  it('draws a structured image as a button that expands and collapses it', async () => {
    render(<InlineImage kind="data" data="AAAA" mimeType="image/png" alt="A plot" />);

    const toggle = screen.getByRole('button', { name: copy.expand });
    expect(screen.getByAltText('A plot')).toHaveAttribute('src', 'data:image/png;base64,AAAA');

    await userEvent.click(toggle);
    expect(screen.getByRole('button', { name: copy.collapse })).toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: copy.collapse }));
    expect(screen.getByRole('button', { name: copy.expand })).toBeInTheDocument();
  });

  it('adds no margin of its own: the block that holds it sets the gap', () => {
    // 8px between blocks is the list's rule (spec 3.6); an image that also
    // carried `my-2` made it 16px under the words.
    const { container } = render(<InlineImage kind="data" data="AAAA" mimeType="image/png" />);
    expect(container.querySelector('.image-preview')?.className).not.toMatch(/\bm[ytb]?-\d/);
  });

  it('refuses a path that is not a pasted image, in the transcript voice', () => {
    render(<InlineImage kind="temp-path" path="/etc/hosts" />);
    expect(screen.getByText(copy.invalidPath('/etc/hosts'))).toHaveClass(
      'text-supporting',
      'text-text-danger'
    );
  });

  it('says so, in the same voice, when a pasted image cannot be read', async () => {
    const path = '/tmp/biorouter-pasted-images/a.png';
    getTempImage.mockResolvedValue(null);
    render(<InlineImage kind="temp-path" path={path} />);
    await waitFor(() => expect(screen.getByText(copy.unavailable(path))).toBeInTheDocument());
  });

  it('shows a still placeholder while a pasted image loads', async () => {
    const path = '/tmp/biorouter-pasted-images/b.png';
    let resolve: (value: string) => void = () => {};
    getTempImage.mockReturnValue(new Promise<string>((r) => (resolve = r)));
    const { container } = render(<InlineImage kind="temp-path" path={path} />);

    expect(screen.getByText(copy.loading)).toBeInTheDocument();
    // A load is not work the eye needs to watch: nothing pulses.
    expect(container.querySelector('[class*="animate-"]')).toBeNull();

    resolve('data:image/png;base64,AAAA');
    await waitFor(() => expect(screen.queryByText(copy.loading)).toBeNull());
  });
});
