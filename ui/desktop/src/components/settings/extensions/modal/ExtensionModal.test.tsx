import { describe, it, expect, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ExtensionModal from './ExtensionModal';
import { ExtensionFormData } from '../utils';
import { EXTENSION_MODAL_COPY as COPY } from '../copy';
import ExtensionTimeoutField from './ExtensionTimeoutField';

describe('ExtensionModal', () => {
  // This is a full render-and-drive integration test: it mounts ExtensionModal,
  // opens a Radix Select, and types into five fields. In jsdom that legitimately
  // takes ~2s on its own, so under the parallel full-suite run (90 test files
  // contending for CPU) it overshoots vitest's default 5s timeout even though it
  // asserts nothing wrong. The explicit timeout below gives it headroom for the
  // loaded case rather than weakening what it verifies; isolated it finishes in ~2s.
  it('creates a http_streamable extension', async () => {
    const user = userEvent.setup();
    const mockOnSubmit = vi.fn();
    const mockOnClose = vi.fn();

    const initialData: ExtensionFormData = {
      name: '',
      description: '',
      type: 'stdio', // Default type
      cmd: '',
      endpoint: '',
      enabled: true,
      timeout: 300,
      envVars: [],
      headers: [],
    };

    render(
      <ExtensionModal
        title="Add custom extension"
        initialData={initialData}
        onClose={mockOnClose}
        onSubmit={mockOnSubmit}
        submitLabel="Add extension"
        modalType="add"
      />
    );

    expect(screen.getByRole('dialog')).not.toHaveAttribute('aria-describedby');

    // Every field has a name a screen reader can say: the labels point at their controls.
    const nameInput = screen.getByLabelText(COPY.nameLabel);
    const submitButton = screen.getByTestId('extension-submit-btn');

    await user.type(nameInput, 'Test MCP');

    // The type is two short options, so it is a segmented control and not a select.
    const typeGroup = screen.getByRole('radiogroup', { name: COPY.typeLabel });
    expect(screen.queryByRole('combobox')).toBeNull();
    await user.click(within(typeGroup).getByRole('radio', { name: COPY.typeHttp }));

    await waitFor(() => {
      expect(screen.getByText(COPY.headersTitle)).toBeInTheDocument();
    });

    const endpointInput = screen.getByLabelText(COPY.endpointLabel);
    await user.type(endpointInput, 'https://foo.bar.com/mcp/');

    const descriptionInput = screen.getByLabelText(COPY.descriptionLabel);
    await user.type(descriptionInput, 'Test MCP extension');

    // The headers group is named by its heading, and its two cells by their own labels.
    const headers = screen.getByRole('group', { name: COPY.headersTitle });
    await user.type(within(headers).getByLabelText(COPY.headerName), 'Authorization');
    await user.type(within(headers).getByLabelText(COPY.value), 'Bearer abc123');

    await user.click(submitButton);

    await waitFor(() => {
      expect(mockOnSubmit).toHaveBeenCalled();
    });

    const submittedData = mockOnSubmit.mock.calls[0][0];

    expect(submittedData.name).toBe('Test MCP');
    expect(submittedData.type).toBe('streamable_http');
    expect(submittedData.endpoint).toBe('https://foo.bar.com/mcp/');
    expect(submittedData.description).toBe('Test MCP extension');
    expect(submittedData.timeout).toBe(300);
    expect(submittedData.headers).toHaveLength(1);
    expect(submittedData.headers).toEqual([
      { key: 'Authorization', value: 'Bearer abc123', isEdited: true },
    ]);
    // 60s, not 20s. This inline per-test timeout WINS over the CLI
    // --testTimeout the CI workflow passes, so a 20s budget here makes the
    // whole gate red regardless of the flag: the test types ~50 characters
    // through user-event at its default inter-event delay and has been measured
    // at 31s on a CI-class runner. The better fix is
    // `userEvent.setup({ delay: null })` in this file; this is the zero-risk
    // version of it.
  }, 60000);

  it('links the visible removal warning only while removal is being confirmed', async () => {
    const user = userEvent.setup({ delay: null });
    const initialData: ExtensionFormData = {
      name: 'developer',
      description: 'Developer tools',
      type: 'stdio',
      cmd: 'developer-server',
      endpoint: '',
      enabled: true,
      timeout: 300,
      envVars: [],
      headers: [],
    };

    render(
      <ExtensionModal
        title="Edit extension"
        initialData={initialData}
        onClose={vi.fn()}
        onSubmit={vi.fn()}
        onDelete={vi.fn()}
        submitLabel="Save"
        modalType="edit"
      />
    );

    const dialog = screen.getByRole('dialog');
    expect(dialog).not.toHaveAttribute('aria-describedby');

    await user.click(screen.getByRole('button', { name: 'Remove extension' }));

    const descriptionId = dialog.getAttribute('aria-describedby');
    expect(descriptionId).toBeTruthy();
    expect(document.getElementById(descriptionId!)).toHaveTextContent(COPY.removeDescription);
    // Sentence case, and the question names what is going.
    expect(screen.getByRole('dialog', { name: COPY.removeTitle('developer') })).toBe(dialog);
  });

  /**
   * Issue #56 §13.5: "The manual 'Add stdio extension' form carries the same
   * line." It is the one install route with no bundle and no catalogue entry
   * behind it, so nothing else on the screen hints at what the result will be.
   *
   * And it follows the NAME as it is typed, because on this form the name is the
   * whole input to the tier. That is DR-19's consequence made visible at the one
   * moment a person can trigger it through the GUI: type a published private
   * name and the badge says Private; type anything else and it says Public.
   * `delay: null` because this types 13 characters and the suite above documents
   * what user-event's default inter-event delay costs.
   */
  it('the add form states the badge the name will produce', async () => {
    const user = userEvent.setup({ delay: null });
    const initialData: ExtensionFormData = {
      name: '',
      description: '',
      type: 'stdio',
      cmd: '',
      endpoint: '',
      enabled: true,
      timeout: 300,
      envVars: [],
      headers: [],
    };

    render(
      <ExtensionModal
        title="Add custom extension"
        initialData={initialData}
        onClose={vi.fn()}
        onSubmit={vi.fn()}
        submitLabel="Add extension"
        modalType="add"
      />
    );

    // §13.5: the Add form's tier line is on screen as text, not behind a glyph.
    expect(screen.getByText(COPY.tierPublic)).toBeVisible();
    expect(
      screen.getByText(/including commercial models hosted outside your institution/i)
    ).toBeInTheDocument();

    await user.type(screen.getByLabelText(COPY.nameLabel), 'ucsfomopagent');

    expect(await screen.findByText(/only private models/i)).toBeInTheDocument();
    expect(screen.getByText(COPY.tierPrivate)).toBeVisible();
    expect(screen.queryByText(/always Public/i)).toBeNull();
  });
});

it('discloses retained credentials and offers separate review before extension removal', async () => {
  const onDelete = vi.fn();
  render(
    <ExtensionModal
      title="Edit extension"
      initialData={{
        name: 'synthetic',
        description: '',
        type: 'stdio',
        cmd: 'unused',
        enabled: false,
        timeout: 300,
        envVars: [],
        headers: [],
      }}
      onClose={vi.fn()}
      onSubmit={vi.fn()}
      onDelete={onDelete}
      submitLabel="Save"
      modalType="edit"
    />
  );
  await userEvent.click(screen.getByRole('button', { name: 'Remove extension' }));
  expect(
    screen.getByText(/Saved credentials are retained and may be reused automatically/)
  ).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Review saved credentials' })).toBeInTheDocument();
  expect(onDelete).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole('button', { name: 'Confirm removal' }));
  expect(onDelete).toHaveBeenCalledWith('synthetic');
});

const stdio = (overrides: Partial<ExtensionFormData> = {}): ExtensionFormData => ({
  name: 'synthetic',
  description: '',
  type: 'stdio',
  cmd: 'unused',
  enabled: true,
  timeout: 300,
  envVars: [],
  headers: [],
  ...overrides,
});

describe('ExtensionModal — the dialog and its form', () => {
  const edit = (props: Partial<React.ComponentProps<typeof ExtensionModal>> = {}) =>
    render(
      <ExtensionModal
        title="Update extension"
        initialData={stdio()}
        onClose={vi.fn()}
        onSubmit={vi.fn()}
        onDelete={vi.fn()}
        submitLabel="Save changes"
        modalType="edit"
        {...props}
      />
    );

  it('is a 640px dialog anchored at the top, with a plain title and no icon before it', () => {
    edit();
    const dialog = screen.getByRole('dialog', { name: 'Update extension' });

    expect(dialog).toHaveAttribute('data-size', 'lg');
    expect(dialog).toHaveAttribute('data-anchor', 'top');
    // `text-iconStandard` was a class with no definition; no other dialog puts an icon in its title.
    expect(dialog.querySelector('[data-slot="dialog-title"] svg')).toBeNull();
  });

  it('gives every text field a name and calls the timeout what it measures', () => {
    edit();
    const dialog = screen.getByRole('dialog');

    for (const field of within(dialog).getAllByRole('textbox')) {
      expect(field).toHaveAccessibleName();
    }
    expect(within(dialog).getByLabelText(COPY.timeoutLabel)).toHaveValue('300');
    expect(COPY.timeoutLabel).toBe('Timeout (seconds)');
  });

  it('puts the long help behind InfoTips and leaves the section titles in sentence case', () => {
    edit();
    const dialog = screen.getByRole('dialog');

    // The paragraph that opened the credentials row is the Review button's InfoTip now.
    expect(within(dialog).getByText(COPY.credentialsHelp)).toHaveClass('sr-only');
    expect(within(dialog).getByText(COPY.envHelp)).toHaveClass('sr-only');
    expect(within(dialog).getByText(COPY.envTitle)).toBeInTheDocument();
    for (const title of [COPY.envTitle, COPY.headersTitle, COPY.installationNotes]) {
      expect(title).toBe(title.charAt(0) + title.slice(1).toLowerCase());
    }
  });

  it('keeps Remove extension destructive, and Cancel quiet', () => {
    edit();
    const remove = screen.getByRole('button', { name: COPY.removeAction });
    const cancel = screen.getByRole('button', { name: COPY.cancel });

    expect(remove.className).toMatch(/text-text-danger/);
    expect(remove.className).not.toMatch(/border-border-emphasized/);
    expect(cancel.className).toMatch(/bg-background-medium/);
    // The Button base already gaps its icon; the old `mr-2` doubled it.
    expect(remove.className).not.toMatch(/\bmr-2\b/);
  });

  it('shows installation notes as an info Note under a sentence-case title', () => {
    render(
      <ExtensionModal
        title="Add custom extension"
        initialData={stdio({ installation_notes: 'Run `uv sync` once before the first start.' })}
        onClose={vi.fn()}
        onSubmit={vi.fn()}
        submitLabel="Add extension"
        modalType="add"
      />
    );

    const title = screen.getByText(COPY.installationNotes);
    expect(title).toBeVisible();
    expect(title.closest('[class*="bg-wash-info"]')).not.toBeNull();
    expect(screen.getByText(/uv sync/)).toBeVisible();
  });

  it('opens at the removal question from the row, and Cancel closes instead of showing a form', async () => {
    const user = userEvent.setup({ delay: null });
    const onClose = vi.fn();
    edit({ startAtRemoval: true, onClose });

    expect(screen.getByRole('dialog', { name: COPY.removeTitle('synthetic') })).toBeInTheDocument();
    expect(screen.queryByLabelText(COPY.nameLabel)).toBeNull();

    await user.click(screen.getByRole('button', { name: COPY.cancel }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('returns from the removal question to the form when it came from the form', async () => {
    const user = userEvent.setup({ delay: null });
    const onClose = vi.fn();
    edit({ onClose });

    await user.click(screen.getByRole('button', { name: COPY.removeAction }));
    await user.click(screen.getByRole('button', { name: COPY.cancel }));

    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByLabelText(COPY.nameLabel)).toBeInTheDocument();
  });

  it('guards unsaved work in sentence case and offers to discard it', async () => {
    const user = userEvent.setup({ delay: null });
    const onClose = vi.fn();
    edit({ onClose });

    await user.type(screen.getByLabelText(COPY.commandLabel), ' --flag');
    await user.click(screen.getByRole('button', { name: COPY.cancel }));

    expect(await screen.findByText(COPY.unsavedTitle)).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: COPY.discard }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('names a retired SSE type in a disabled third segment instead of dropping it', () => {
    edit({ initialData: stdio({ type: 'sse', endpoint: 'https://example.com/sse', cmd: '' }) });

    expect(screen.getByRole('radio', { name: COPY.typeSse })).toBeDisabled();
  });

  it('names the edit and remove buttons on a saved variable and a header row', () => {
    edit({
      initialData: stdio({ envVars: [{ key: 'API_TOKEN', value: '••••••••', isEdited: false }] }),
    });

    expect(screen.getByRole('button', { name: COPY.edit('API_TOKEN') })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: COPY.removeEntry('API_TOKEN') })).toBeInTheDocument();
  });
});

describe('ExtensionTimeoutField', () => {
  it('says the whole sentence, in the field’s own flow, when the timeout is not a positive number', () => {
    render(<ExtensionTimeoutField timeout={0} onChange={vi.fn()} submitAttempted />);

    const input = screen.getByLabelText(COPY.timeoutLabel);
    const message = screen.getByText(COPY.timeoutRequired);
    // It used to read "Timeout " and was `absolute`, floating over whatever followed.
    expect(message.className).not.toMatch(/\babsolute\b/);
    expect(input).toHaveAttribute('aria-invalid', 'true');
    expect(input).toHaveAccessibleDescription(COPY.timeoutRequired);
  });

  it('is quiet until a submit was attempted', () => {
    render(<ExtensionTimeoutField timeout={0} onChange={vi.fn()} submitAttempted={false} />);
    expect(screen.queryByText(COPY.timeoutRequired)).toBeNull();
  });
});
