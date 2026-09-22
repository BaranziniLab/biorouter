import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { DisplayItem, getMentionInsertText, mentionReference } from './MentionPopover';
import { findRefTags } from '../utils/resourceRefs';

const item = (overrides: Partial<DisplayItem>): DisplayItem => ({
  name: 'example',
  extra: 'Extra detail',
  itemType: 'File',
  relativePath: 'example',
  ...overrides,
});

// The same corpus the two parser implementations are pinned to. A name the
// backend contract can carry is a name the popover has to be able to insert.
const CORPUS: [string, string][] = JSON.parse(
  readFileSync(
    path.join(process.cwd(), '..', '..', 'crates/biorouter/src/agents/resource_ref_corpus.json'),
    'utf8'
  )
).pairs;

/**
 * What the agent will read back out of an inserted reference.
 *
 * Asserting on this rather than on the marker's spelling is the whole point of
 * issue #65: the old marker looked perfectly well formed and named the wrong
 * resource. The shape of the insert is an implementation detail; that the agent
 * resolves what the user picked is the contract.
 */
const readBack = (inserted: string) => findRefTags(inserted).map((ref) => [ref.kind, ref.value]);

describe('getMentionInsertText', () => {
  it('inserts a resolvable reference for skills and extensions', () => {
    expect(
      readBack(
        getMentionInsertText(
          item({
            name: 'skill:literature-review',
            itemType: 'Skill',
            relativePath: 'literature-review',
          })
        )
      )
    ).toEqual([['skill', 'literature-review']]);

    expect(
      readBack(
        getMentionInsertText(
          item({ name: 'ext:pubmed', itemType: 'Extension', relativePath: 'pubmed' })
        )
      )
    ).toEqual([['extension', 'pubmed']]);
  });

  it('inserts a resolvable reference for knowledge bases', () => {
    expect(
      readBack(
        getMentionInsertText(
          item({
            name: 'kb:Project notes',
            itemType: 'KnowledgeBase',
            relativePath: 'project-notes',
          })
        )
      )
    ).toEqual([['knowledge_base', 'project-notes']]);
  });

  // Issue #65. `extract_inline_refs` splits the message on whitespace, so
  // `/skill:my skill` reached the resolver as `my`. `loadSkill` looks a skill up
  // *exactly*, so the #60 fix for `/ext:` — drop the whitespace — cannot
  // transfer either: `myskill` is not `my skill`. The tag delimits the value
  // explicitly instead of by whitespace, which is why it is now the form the
  // composer emits.
  it('carries a skill name containing a space', () => {
    const inserted = getMentionInsertText(
      item({ name: 'skill:my skill', itemType: 'Skill', relativePath: 'my skill' })
    );

    expect(readBack(inserted)).toEqual([['skill', 'my skill']]);
  });

  it('carries every name the backend contract admits', () => {
    for (const [name] of CORPUS) {
      if (name.trim() === '') continue;
      const inserted = getMentionInsertText(
        item({ name: `skill:${name}`, itemType: 'Skill', relativePath: name })
      );

      expect(readBack(inserted), `name ${JSON.stringify(name)}`).toEqual([['skill', name]]);
    }
  });

  // Issue #60 dropped the whitespace from an extension name because the compact
  // marker could not carry it — lossless only because every `/ext:` consumer
  // re-normalises. The tag carries the name exactly, so nothing has to be
  // dropped and the reference no longer rests on that coincidence.
  describe('extension names', () => {
    const extensionInsert = (name: string) =>
      getMentionInsertText(
        item({ name: `ext:${name}`, itemType: 'Extension', relativePath: name })
      );

    it('carries a bundled extension whose name has a space, unaltered', () => {
      expect(readBack(extensionInsert('Chat Recall'))).toEqual([['extension', 'Chat Recall']]);
      expect(readBack(extensionInsert('Extension Manager'))).toEqual([
        ['extension', 'Extension Manager'],
      ]);
    });

    it('leaves an extension name without whitespace exactly as configured', () => {
      for (const name of ['pubmed', 'agent_drafter', 'my-lab-tools']) {
        expect(readBack(extensionInsert(name))).toEqual([['extension', name]]);
      }
    });

    // `_` and `-` survive the backend's `normalize` and so are part of a user
    // extension's key. Mapping them away — the tempting "normalise to an id"
    // fix — would stop matching an extension stored under `my_tool`.
    it('keeps separators that are part of a user extension key', () => {
      for (const name of ['my_tool', 'my-tool', 'my_lab tools']) {
        expect(readBack(extensionInsert(name))).toEqual([['extension', name]]);
      }
    });

    // A user extension may be named like a bundled one. Which of the two wins is
    // the backend resolver's call; the popover must not change the answer by
    // rewriting the name it was configured with.
    it('leaves a user extension colliding with a bundled id spelled as configured', () => {
      for (const name of ['chat_recall', 'chat-recall', 'Chat Recall']) {
        expect(readBack(extensionInsert(name))).toEqual([['extension', name]]);
      }
    });
  });

  it('keeps backend slash commands executable when selected by keyboard', () => {
    expect(
      getMentionInsertText(item({ name: 'compact', itemType: 'Builtin', relativePath: 'compact' }))
    ).toBe('/compact');
  });

  it('uses a routed reference for UI-only resource commands', () => {
    expect(
      readBack(
        getMentionInsertText(
          item({ name: 'knowledge', itemType: 'Builtin', relativePath: 'knowledge' })
        )
      )
    ).toEqual([['extension', 'knowledge']]);
  });

  it('leaves a file mention as the path it always was', () => {
    expect(getMentionInsertText(item({ itemType: 'File', extra: '/w/notes.md' }))).toBe(
      '/w/notes.md'
    );
  });
});

describe('mentionReference', () => {
  // The composer attaches the reference to its chip rail rather than splicing
  // markup into the textarea, so it needs the resource itself, not a string to
  // paste.
  it('describes the resource a reference item names', () => {
    expect(
      mentionReference(
        item({ itemType: 'Skill', name: 'skill:my skill', relativePath: 'my skill' })
      )
    ).toEqual({ kind: 'skill', value: 'my skill', label: undefined });

    expect(
      mentionReference(
        item({ itemType: 'Extension', name: 'ext:Chat Recall', relativePath: 'Chat Recall' })
      )
    ).toEqual({ kind: 'extension', value: 'Chat Recall', label: undefined });
  });

  // A knowledge base is picked by name and resolved by id, so the chip needs
  // both: the slug is what `kb_search` takes, the name is what the user chose.
  it('carries a knowledge base display name alongside its id', () => {
    expect(
      mentionReference(
        item({ itemType: 'KnowledgeBase', name: 'kb:Project notes', relativePath: 'project-notes' })
      )
    ).toEqual({ kind: 'knowledge_base', value: 'project-notes', label: 'Project notes' });
  });

  it('names the extension reachable through the /knowledge convenience command', () => {
    expect(
      mentionReference(item({ itemType: 'Builtin', name: 'knowledge', relativePath: 'knowledge' }))
    ).toEqual({ kind: 'extension', value: 'knowledge', label: undefined });
  });

  it('does not reinterpret a resource or workflow named like a client command', () => {
    expect(
      mentionReference(item({ itemType: 'Skill', name: 'knowledge', relativePath: 'knowledge' }))
    ).toEqual({ kind: 'skill', value: 'knowledge', label: undefined });
    expect(
      mentionReference(item({ itemType: 'Workflow', name: 'knowledge', relativePath: 'knowledge' }))
    ).toBeNull();
    expect(
      getMentionInsertText(
        item({ itemType: 'Workflow', name: 'knowledge', relativePath: 'knowledge' })
      )
    ).toBe('/knowledge');
  });

  it('is null for anything that is not a resource reference', () => {
    expect(mentionReference(item({ itemType: 'File' }))).toBeNull();
    expect(
      mentionReference(item({ itemType: 'Builtin', name: 'compact', relativePath: 'compact' }))
    ).toBeNull();
    expect(
      mentionReference(item({ itemType: 'Workflow', name: 'nightly', relativePath: 'nightly' }))
    ).toBeNull();
  });
});
