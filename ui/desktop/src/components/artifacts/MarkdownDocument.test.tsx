import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import MarkdownDocument from './MarkdownDocument';

vi.mock('../MarkdownContent', () => ({
  default: ({ content }: { content: string }) => <div>{content}</div>,
}));

describe('MarkdownDocument front matter', () => {
  it('shows a subtitle when no title or other header fields exist', () => {
    render(
      <MarkdownDocument path="/report.md" text={'---\nsubtitle: Interim results\n---\nBody'} />
    );
    expect(screen.getByText('Interim results')).toBeInTheDocument();
    expect(screen.getByText('Body')).toBeInTheDocument();
  });
});
