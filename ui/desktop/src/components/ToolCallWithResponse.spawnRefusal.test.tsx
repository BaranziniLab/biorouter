import { afterEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import ToolCallWithResponse from './ToolCallWithResponse';
import type { ToolRequestMessageContent, ToolResponseMessageContent } from '../types/message';
import { BROWSER_SURFACE_MARKER } from '../utils/surface';
import { MAKE_CHAT_PUBLIC_FROM_HISTORY } from './subagent/spawnRefusalHint';

const noopOpenArtifact = vi.fn();

/**
 * Owner decision 4 (2026-11-12): a private chat refused a subagent on a public
 * model is told, under the refusal and to the person alone, that the chat can be
 * made public from History. The agent's refusal is unchanged and never carries
 * the line; `subagent_tool_person_hint_tests.rs` holds that half.
 */

/** DR-19's refusal (`PrivacyRefusal::PublicChildOfPrivateParent`), as the daemon writes it. */
const DR19 =
  'This chat is private, so it cannot start a subagent on a public model: the task prompt is ' +
  'private-origin text, sending it would put that text on a model outside the institution, and ' +
  'this request came from the assistant rather than from the person at the keyboard. No ' +
  'subagent was started and this chat is unchanged. Do not retry; the same call will be ' +
  'refused again, and no setting, hook or permission mode changes it. If the task really ' +
  'belongs on that model, tell the user so: they can start a new chat on it and give it the ' +
  'task directly.';

const HEADER =
  '[tool_error kind=invalid_args retryable=false] The arguments were rejected. Fix the call ' +
  'itself; the same arguments will fail again.';

/** A refused spawn as a live chat stores it: the taxonomy's header, then the sentence. */
function liveWire(sentence: string): Record<string, unknown> {
  const envelope = { error: { kind: 'invalid_args', retryable: false, message: sentence } };
  return {
    status: 'error',
    error: `-32602: ${HEADER}\n${sentence}(${JSON.stringify(envelope)})`,
    error_data: { code: -32602, message: `${HEADER}\n${sentence}`, data: envelope },
    error_kind: 'invalid_args',
    retryable: false,
  };
}

function renderRefusal({
  toolName = 'workspace__subagent',
  toolResult = liveWire(DR19),
  sessionId = 'chat-private-1',
}: {
  toolName?: string;
  toolResult?: Record<string, unknown>;
  sessionId?: string;
} = {}) {
  const toolRequest: ToolRequestMessageContent = {
    type: 'toolRequest',
    id: 'tool-refused-spawn',
    toolCall: {
      status: 'success',
      value: {
        name: toolName,
        arguments: { instructions: 'Read every trial report.', role: 'Reader' },
      },
    },
  };
  const toolResponse: ToolResponseMessageContent = {
    type: 'toolResponse',
    id: 'tool-refused-spawn',
    toolResult,
  };
  render(
    <ToolCallWithResponse
      isCancelledMessage={false}
      sessionId={sessionId}
      toolRequest={toolRequest}
      toolResponse={toolResponse}
      onOpenArtifact={noopOpenArtifact}
    />
  );
  const row = screen
    .getAllByRole('button')
    .find((button) => button.getAttribute('aria-expanded') !== null);
  fireEvent.click(row as HTMLElement);
}

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

describe("a private chat's refused public subagent", () => {
  it('reads the sentence, then the person-only line', () => {
    renderRefusal();
    const notice = screen.getByRole('alert');
    expect(notice).toHaveTextContent(DR19);
    // No machine text: the code, the taxonomy's header or the envelope's JSON.
    for (const raw of ['-32602', '[tool_error', 'retryable', '{"error"']) {
      expect(notice).not.toHaveTextContent(raw);
    }
    const hint = screen.getByText(MAKE_CHAT_PUBLIC_FROM_HISTORY);
    // A plain line beside the refusal, not part of the error's own words.
    expect(notice).not.toContainElement(hint);
    expect(hint.className).toContain('text-text-muted');
    expect(hint.className).not.toMatch(/danger|font-mono/);
  });

  it('adds the line under the bare tool name and a chat saved with the old wrapper', () => {
    renderRefusal({
      toolName: 'subagent',
      toolResult: { status: 'error', error: `\`workspace__subagent\` failed: -32602: ${DR19}` },
    });
    expect(screen.getByRole('alert')).toHaveTextContent(DR19);
    expect(screen.getByText(MAKE_CHAT_PUBLIC_FROM_HISTORY)).toBeInTheDocument();
  });

  it('keeps the line out of a saved or shared transcript, which has no chat to change', () => {
    renderRefusal({ sessionId: '' });
    expect(screen.getByRole('alert')).toHaveTextContent(DR19);
    expect(screen.queryByText(MAKE_CHAT_PUBLIC_FROM_HISTORY)).toBeNull();
  });

  it('keeps the line out of a browser, which cannot make a chat public', () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    renderRefusal();
    expect(screen.getByRole('alert')).toHaveTextContent(DR19);
    expect(screen.queryByText(MAKE_CHAT_PUBLIC_FROM_HISTORY)).toBeNull();
  });

  it('adds nothing under the other spawn refusals or another tool printing the words', () => {
    const r4 =
      'This chat is running on a public model, so it cannot delegate to a private one: a ' +
      'subagent may never reach further than the chat that started it.';
    renderRefusal({ toolResult: liveWire(r4) });
    expect(screen.getByRole('alert')).toHaveTextContent(r4);
    expect(screen.queryByText(MAKE_CHAT_PUBLIC_FROM_HISTORY)).toBeNull();
  });

  it('adds nothing when a third-party tool prints the refusal', () => {
    renderRefusal({ toolName: 'someext__subagent' });
    expect(screen.queryByText(MAKE_CHAT_PUBLIC_FROM_HISTORY)).toBeNull();
  });
});

describe("a failed call's text when the data is not only Biorouter's", () => {
  it("keeps the display, so a tool's own explanation still shows", () => {
    renderRefusal({
      toolName: 'someext__lookup',
      toolResult: {
        status: 'error',
        error: '-32000: Lookup failed({"detail":"table cohort_3 is locked by a nightly job"})',
        error_data: {
          code: -32000,
          message: 'Lookup failed',
          data: { detail: 'table cohort_3 is locked by a nightly job' },
        },
      },
    });
    expect(screen.getByRole('alert')).toHaveTextContent('locked by a nightly job');
  });
});
