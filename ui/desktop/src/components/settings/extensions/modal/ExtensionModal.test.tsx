import { describe, it, expect, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ExtensionModal from './ExtensionModal';
import { ExtensionFormData } from '../utils';

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

    const nameInput = screen.getByPlaceholderText('Enter extension name...');
    const submitButton = screen.getByTestId('extension-submit-btn');

    await user.type(nameInput, 'Test MCP');

    const typeSelect = screen.getByRole('combobox');
    await user.click(typeSelect);

    const httpOption = screen.getByText('Streamable HTTP');
    await user.click(httpOption);

    await waitFor(() => {
      expect(screen.getByText('Request Headers')).toBeInTheDocument();
    });

    const endpointInput = screen.getByPlaceholderText('Enter endpoint URL...');
    await user.type(endpointInput, 'https://foo.bar.com/mcp/');

    const descriptionInput = screen.getByPlaceholderText('Optional description...');
    await user.type(descriptionInput, 'Test MCP extension');

    const headerNameInput = screen.getByPlaceholderText('Header name');
    const headerValueInput = screen
      .getAllByPlaceholderText('Value')
      .find(
        (input) =>
          input.closest('div')?.textContent?.includes('Request Headers') ||
          input.parentElement?.parentElement?.textContent?.includes('Request Headers')
      );

    await user.type(headerNameInput, 'Authorization');
    if (headerValueInput) {
      await user.type(headerValueInput, 'Bearer abc123');
    }

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
    expect(document.getElementById(descriptionId!)).toHaveTextContent(
      'This will permanently remove this extension and all of its settings.'
    );
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

    expect(
      screen.getByText(/including commercial models hosted outside your institution/i)
    ).toBeInTheDocument();

    await user.type(screen.getByPlaceholderText('Enter extension name...'), 'ucsfomopagent');

    expect(await screen.findByText(/only private models/i)).toBeInTheDocument();
    expect(screen.queryByText(/always Public/i)).toBeNull();
  });
});
