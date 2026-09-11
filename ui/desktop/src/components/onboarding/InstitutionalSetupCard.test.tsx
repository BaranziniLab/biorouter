import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import InstitutionalSetupCard from './InstitutionalSetupCard';

const mockUpsert = vi.fn();
const mockCheckProvider = vi.fn();

vi.mock('../ConfigContext', () => ({
  useConfig: () => ({ upsert: (...args: unknown[]) => mockUpsert(...args) }),
}));
vi.mock('../../api', () => ({
  checkProvider: (...args: unknown[]) => mockCheckProvider(...args),
}));
vi.mock('react-router-dom', () => ({ useNavigate: () => vi.fn() }));

/** The three keys that belong to the PUBLIC `azure_openai` provider. */
const PUBLIC_AZURE_KEYS = [
  'AZURE_OPENAI_ENDPOINT',
  'AZURE_OPENAI_DEPLOYMENT_NAME',
  'AZURE_OPENAI_API_VERSION',
];

/** Types a key, optionally edits Advanced, and waits for the connect to finish. */
async function connectVersaAzure(advanced: { endpoint?: string; apiVersion?: string } = {}) {
  const onSuccess = vi.fn();
  render(<InstitutionalSetupCard onSuccess={onSuccess} />);
  fireEvent.change(screen.getByLabelText(/API Key/i), { target: { value: 'a-key' } });
  if (advanced.endpoint !== undefined || advanced.apiVersion !== undefined) {
    fireEvent.click(screen.getByRole('button', { name: /Advanced/i }));
  }
  if (advanced.endpoint !== undefined) {
    fireEvent.change(screen.getByDisplayValue('https://unified-api.ucsf.edu/general'), {
      target: { value: advanced.endpoint },
    });
  }
  if (advanced.apiVersion !== undefined) {
    fireEvent.change(screen.getByDisplayValue('2025-01-01-preview'), {
      target: { value: advanced.apiVersion },
    });
  }
  fireEvent.click(screen.getByRole('button', { name: /Connect to Versa Azure OpenAI/i }));
  // Past `checkProvider`, so every write the connect makes has been recorded.
  await waitFor(() => expect(onSuccess).toHaveBeenCalledWith('versa_azure'));
}

const writtenKeys = () => mockUpsert.mock.calls.map((c) => c[0] as string);

describe('InstitutionalSetupCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockUpsert.mockResolvedValue(undefined);
    mockCheckProvider.mockResolvedValue({ data: {} });
  });

  it('never writes the public Azure provider keys when connecting UCSF Versa', async () => {
    // Connecting UCSF's PRIVATE Versa used to write the PUBLIC `azure_openai`
    // card's own three keys, with values identical to the defaults Versa
    // already falls back to. The write changed nothing for Versa and made
    // `check_provider_configured` report a Public provider the user never set
    // up as Configured, one row away in the same grid.
    await connectVersaAzure();
    for (const key of PUBLIC_AZURE_KEYS) {
      expect(writtenKeys()).not.toContain(key);
    }
  });

  it('never writes a deployment: the model a chat picks chooses it', async () => {
    // `versa_azure` posts each model to its own deployment. A configured
    // `VERSA_AZURE_DEPLOYMENT_NAME` that names a catalog deployment is ignored,
    // and any other value pins EVERY model to that one deployment while the
    // chat still shows the model it picked. Writing the shipped default was
    // the first case on every connect; the box that let a user write anything
    // else was the second.
    await connectVersaAzure();
    expect(writtenKeys()).not.toContain('VERSA_AZURE_DEPLOYMENT_NAME');
  });

  it('writes the key, the endpoint and the API version, then selects the provider', async () => {
    await connectVersaAzure();
    expect(mockUpsert.mock.calls).toEqual([
      ['VERSA_AZURE_API_KEY', 'a-key', true],
      ['VERSA_AZURE_ENDPOINT', 'https://unified-api.ucsf.edu/general', false],
      ['VERSA_AZURE_API_VERSION', '2025-01-01-preview', false],
      ['BIOROUTER_PROVIDER', 'versa_azure', false],
    ]);
  });

  it('writes the endpoint and API version typed into Advanced', async () => {
    await connectVersaAzure({
      endpoint: ' https://gateway.example.edu/general ',
      apiVersion: '2024-10-21',
    });
    expect(mockUpsert).toHaveBeenCalledWith(
      'VERSA_AZURE_ENDPOINT',
      'https://gateway.example.edu/general',
      false
    );
    expect(mockUpsert).toHaveBeenCalledWith('VERSA_AZURE_API_VERSION', '2024-10-21', false);
  });

  it('offers the endpoint and API version in Advanced, and no deployment', () => {
    render(<InstitutionalSetupCard onSuccess={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: /Advanced/i }));
    expect(screen.getByText('VERSA_AZURE_ENDPOINT')).toBeInTheDocument();
    expect(screen.getByText('VERSA_AZURE_API_VERSION')).toBeInTheDocument();
    // Neither a field nor a mention in the collapsed label.
    expect(screen.queryAllByText(/deployment/i)).toHaveLength(0);
  });
});
