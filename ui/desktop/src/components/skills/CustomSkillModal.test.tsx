import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { CatalogSkill } from '../../api';
import CustomSkillModal from './CustomSkillModal';
import { CUSTOM_SKILL_COPY } from './copy';

afterEach(cleanup);

const state = {
  machineEnabled: true,
  session: 'default' as const,
  sessionViaBundle: false,
  hiddenContext: false,
  effective: true,
};

/** A catalog row as the daemon reports it, in Biorouter's own skills root unless overridden. */
function installed(name: string, overrides: Partial<CatalogSkill> = {}): CatalogSkill {
  return {
    name,
    description: `${name} does things`,
    slug: name,
    directory: `/home/me/.config/biorouter/skills/${name}`,
    sourceRoot: '/home/me/.config/biorouter/skills',
    source: { kind: 'biorouter', extension: null, label: 'Biorouter' },
    bundle: null,
    builtin: false,
    state,
    ...overrides,
  };
}

const skillMd = (name: string) =>
  `---\nname: ${name}\ndescription: Mine, written by hand.\n---\n\n# ${name}\n`;

function stubWrites() {
  const electron = {
    ensureDirectory: vi.fn().mockResolvedValue(true),
    writeFile: vi.fn().mockResolvedValue(true),
  };
  (window as unknown as { electron: unknown }).electron = electron;
  return electron;
}

async function saveAs(name: string) {
  const user = userEvent.setup();
  const editor = screen.getByRole('textbox', { name: CUSTOM_SKILL_COPY.editorLabel });
  fireEvent.change(editor, { target: { value: skillMd(name) } });
  await user.click(screen.getByRole('button', { name: CUSTOM_SKILL_COPY.save }));
}

describe('CustomSkillModal', () => {
  it('blocks all dismissal while the skill is being written', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    let finishWrite: (value: boolean) => void = () => {};
    const pendingWrite = new Promise<boolean>((resolve) => {
      finishWrite = resolve;
    });

    (window as unknown as { electron: unknown }).electron = {
      ensureDirectory: vi.fn().mockResolvedValue(undefined),
      writeFile: vi.fn().mockReturnValue(pendingWrite),
    };

    render(<CustomSkillModal skills={[]} onClose={onClose} onSaved={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Save skill' }));

    expect(await screen.findByRole('button', { name: 'Saving…' })).toBeDisabled();
    expect(screen.queryByRole('button', { name: 'Close' })).not.toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();

    finishWrite(false);
    await waitFor(() => expect(screen.getByRole('button', { name: 'Save skill' })).toBeEnabled());
  });
});

describe('CustomSkillModal: the guidance is help on demand', () => {
  it('shows no sentence, and gives the editor its requirement and its location as a description', () => {
    render(<CustomSkillModal skills={[]} onClose={() => {}} onSaved={() => {}} />);

    // Nothing but the label is visible: the requirement and the location are the help.
    expect(screen.queryByText(CUSTOM_SKILL_COPY.help, { selector: ':not(.sr-only)' })).toBeNull();
    const editor = screen.getByRole('textbox', { name: CUSTOM_SKILL_COPY.editorLabel });
    expect(editor).toHaveAccessibleDescription(CUSTOM_SKILL_COPY.help);
    expect(
      screen.getByRole('button', { name: `About ${CUSTOM_SKILL_COPY.editorLabel}` })
    ).toHaveAccessibleDescription(CUSTOM_SKILL_COPY.help);
  });

  it('puts first focus in the editor, not on the help glyph', async () => {
    render(<CustomSkillModal skills={[]} onClose={() => {}} onSaved={() => {}} />);
    await waitFor(() =>
      expect(screen.getByRole('textbox', { name: CUSTOM_SKILL_COPY.editorLabel })).toHaveFocus()
    );
  });

  it('is anchored at the top, so a refused save grows it downward', () => {
    render(<CustomSkillModal skills={[]} onClose={() => {}} onSaved={() => {}} />);
    expect(screen.getByRole('dialog')).toHaveAttribute('data-anchor', 'top');
    expect(screen.getByRole('dialog')).toHaveAttribute('data-size', 'lg');
  });

  it('says what is missing when the frontmatter is not there', async () => {
    const user = userEvent.setup();
    render(<CustomSkillModal skills={[]} onClose={() => {}} onSaved={() => {}} />);
    const editor = screen.getByRole('textbox', { name: CUSTOM_SKILL_COPY.editorLabel });
    await user.clear(editor);
    await user.type(editor, 'no frontmatter');
    await user.click(screen.getByRole('button', { name: CUSTOM_SKILL_COPY.save }));
    expect(await screen.findByRole('alert')).toHaveTextContent(CUSTOM_SKILL_COPY.invalid);
  });
});

// A save writes `<skills>/<slug>/SKILL.md` through the main process, which overwrites whatever is
// there. Saving `office-word` replaced Biorouter's shipped file until the seeder restored it, and
// the person's text was lost without a word (rq2-247).
describe('CustomSkillModal: a name that is taken is refused before anything is written', () => {
  it('saving_a_shipped_skill_name_is_refused_and_writes_nothing', async () => {
    const electron = stubWrites();
    const onSaved = vi.fn();
    const onClose = vi.fn();
    const skills = [
      installed('office-word', { builtin: true }),
      // A shipped bundle: its folder is the bundle's, so a skill named after it would land inside.
      installed('okf-ingest', {
        builtin: true,
        bundle: 'knowledge-bases',
        slug: 'knowledge-bases/okf-ingest',
        directory: '/home/me/.config/biorouter/skills/knowledge-bases/okf-ingest',
      }),
    ];
    render(<CustomSkillModal skills={skills} onClose={onClose} onSaved={onSaved} />);

    await saveAs('office-word');
    expect(await screen.findByRole('alert')).toHaveTextContent(
      CUSTOM_SKILL_COPY.shipped('office-word')
    );
    await saveAs('Knowledge Bases');
    expect(await screen.findByRole('alert')).toHaveTextContent(
      CUSTOM_SKILL_COPY.shipped('Knowledge Bases')
    );

    expect(electron.ensureDirectory).not.toHaveBeenCalled();
    expect(electron.writeFile).not.toHaveBeenCalled();
    expect(onSaved).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('saving_an_existing_name_is_refused', async () => {
    const electron = stubWrites();
    const skills = [
      installed('my-notes'),
      // Another root: no folder is overwritten, but one name would shadow the other.
      installed('lab-style', {
        directory: '/home/me/.claude/skills/lab-style',
        sourceRoot: '/home/me/.claude/skills',
        source: { kind: 'claudeHome', extension: null, label: 'Claude' },
      }),
    ];
    render(<CustomSkillModal skills={skills} onClose={() => {}} onSaved={() => {}} />);

    // The folder `my-notes` is the same one, however the name is written.
    await saveAs('My Notes');
    expect(await screen.findByRole('alert')).toHaveTextContent(
      CUSTOM_SKILL_COPY.exists('My Notes')
    );
    await saveAs('lab-style');
    expect(await screen.findByRole('alert')).toHaveTextContent(
      CUSTOM_SKILL_COPY.exists('lab-style')
    );
    expect(electron.writeFile).not.toHaveBeenCalled();

    // A name nothing holds still saves, so the refusal is about the name and nothing else.
    await saveAs('fresh-idea');
    await waitFor(() =>
      expect(electron.writeFile).toHaveBeenCalledWith(
        expect.stringMatching(/\/fresh-idea\/SKILL\.md$/),
        skillMd('fresh-idea')
      )
    );
  });

  it('refuses to save when the installed skills could not be read', async () => {
    const electron = stubWrites();
    render(<CustomSkillModal skills={null} onClose={() => {}} onSaved={() => {}} />);
    await saveAs('anything');
    expect(await screen.findByRole('alert')).toHaveTextContent(CUSTOM_SKILL_COPY.cannotCheck);
    expect(electron.writeFile).not.toHaveBeenCalled();
  });
});
