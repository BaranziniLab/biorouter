import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Message } from '../api';

/** Every set of props a MarkdownContent was rendered with, in order. */
const rendered = vi.hoisted(() => ({ props: [] as Array<Record<string, unknown>> }));

// The real renderer, behind a recorder that sees each render's props. The recorder is not memo'd,
// so it records every time BioRouterMessage renders; the real (memo'd) MarkdownContent behind it
// re-renders only when those props change.
vi.mock('./MarkdownContent', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./MarkdownContent')>();
  const Real = actual.default;
  function RecordingMarkdownContent(props: ComponentProps<typeof Real>) {
    rendered.props.push(props as unknown as Record<string, unknown>);
    return <Real {...props} />;
  }
  return { default: RecordingMarkdownContent };
});

import BioRouterMessage from './BioRouterMessage';

const noopOpenArtifact = vi.fn();

function assistant(id: string, text: string): Message {
  return {
    id,
    role: 'assistant',
    created: 1700000000000,
    metadata: { userVisible: true, agentVisible: true },
    content: [{ type: 'text', text }],
  };
}

// An earlier message names a file, so the finished message's lookup has something to answer.
const earlier = assistant('m0', 'I wrote `/work/report.md`.');
const finished = assistant('m1', 'Run it:\n\n```python\nprint("hi")\n```\n\nSee `report.md`.');

function view(messages: Message[]) {
  return (
    <BioRouterMessage
      sessionId="s1"
      message={finished}
      messages={messages}
      messageIndex={1}
      toolCallNotifications={new Map()}
      onOpenArtifact={noopOpenArtifact}
      onRunInTerminal={null}
      workingDir="/work"
    />
  );
}

/** The props the finished message's own text was last rendered with. */
function lastBodyProps() {
  const body = rendered.props.filter((props) => String(props.content).includes('print("hi")'));
  return body[body.length - 1];
}

/**
 * The reported bug: Copy on a code block in the chat "sometimes" did nothing. It failed while the
 * NEXT reply streamed. Every chunk is a new `messages` array, and the file lookup each message
 * hands its MarkdownContent was rebuilt from it — a new function per chunk for every message in
 * the transcript, which re-rendered each one's markdown on every token (and, with the old inline
 * renderer map, rebuilt its fenced blocks mid-click).
 */
describe('BioRouterMessage — a finished message while the next reply streams', () => {
  beforeEach(() => {
    rendered.props = [];
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      writable: true,
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
    });
  });

  it('hands MarkdownContent the same file lookup for every streamed chunk', () => {
    const { rerender } = render(view([earlier, finished]));
    const first = lastBodyProps().knownFilePaths as (name: string) => readonly string[];
    expect(first('report.md')).toEqual(['/work/report.md']);

    rerender(view([earlier, finished, assistant('m2', 'Wor')]));
    rerender(view([earlier, finished, assistant('m2', 'Working on it')]));

    expect(lastBodyProps().knownFilePaths).toBe(first);
  });

  it('keeps the same Copy button, and its "Copied", through streamed chunks', async () => {
    const { container, rerender } = render(view([earlier, finished]));
    await waitFor(() => expect(container.querySelector('.biorouter-md-code')).not.toBeNull());
    const button = screen.getByRole('button', { name: 'Copy' });

    fireEvent.click(button);
    await waitFor(() => expect(button).toHaveTextContent('Copied'));

    rerender(view([earlier, finished, assistant('m2', 'Wor')]));
    rerender(view([earlier, finished, assistant('m2', 'Working on it')]));

    expect(screen.getByRole('button', { name: 'Copied' })).toBe(button);
  });

  it('still answers a newly named file once the messages before it change', () => {
    const { rerender } = render(view([earlier, finished]));
    const first = lastBodyProps().knownFilePaths as (name: string) => readonly string[];

    const renamed = assistant('m0', 'I wrote `/work/other/report.md`.');
    rerender(view([renamed, finished]));

    const next = lastBodyProps().knownFilePaths as (name: string) => readonly string[];
    expect(next).not.toBe(first);
    expect(next('report.md')).toEqual(['/work/other/report.md']);
  });
});
