import { describe, expect, it } from 'vitest';
import type { Message } from '../../api';
import { subagentTranscript } from './subagentTranscript';

const message = (
  id: string,
  role: 'user' | 'assistant',
  text: string,
  provenance?: string
): Message =>
  ({
    id,
    role,
    created: 0,
    content: [{ type: 'text', text }],
    metadata: {
      userVisible: true,
      agentVisible: true,
      ...(provenance ? { provenance: { kind: provenance } } : {}),
    },
  }) as unknown as Message;

describe("a subagent tab's transcript", () => {
  // R2-4: "Begin." drew as the person's message, with an edit pencil.
  it('drops the spawn record and the synthetic "Begin." prompt', () => {
    const shown = subagentTranscript([
      message('spawn', 'user', '## Spawn context', 'spawn_context'),
      message('begin', 'user', 'Begin.'),
      message('work', 'assistant', 'Reading the note.'),
      message('later', 'user', 'Begin.'),
    ]);
    expect(shown.map((m) => m.id)).toEqual(['work', 'later']);
  });

  it('keeps a first prompt that carries a person’s steering', () => {
    const shown = subagentTranscript([
      message(
        'begin',
        'user',
        'Begin.\n\nHuman steering received before this run started:\nUse note2'
      ),
      message('work', 'assistant', 'OK'),
    ]);
    expect(shown.map((m) => m.id)).toEqual(['begin', 'work']);
  });
});
