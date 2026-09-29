import React, { useState, useEffect } from 'react';
import { Input } from '../../../../../ui/input';
import { SecretInput } from '../../../../../ui/secret-input';
import { Select } from '../../../../../ui/Select';
import { Button } from '../../../../../ui/button';
import { SecureStorageNotice } from '../SecureStorageNotice';
import { Checkbox } from '@radix-ui/themes';
import { UpdateCustomProviderRequest } from '../../../../../../api';
import { isBrowserSurface } from '../../../../../../utils/surface';
import { HOST_MANAGED_CUSTOM_URL } from '../../../../../privacy/hostManagedModelCopy';

/**
 * What to show when the save was refused. The daemon's refusal is a sentence
 * written for a person (a key the provider rejected, a URL move it will not
 * make), and the generated client throws that parsed body, a string, under
 * `throwOnError`.
 */
export function customProviderSaveFailure(error: unknown): string {
  if (typeof error === 'string' && error.trim()) return error.trim();
  if (error instanceof Error && error.message.trim()) return error.message.trim();
  return 'The provider was not saved. Try again.';
}

interface CustomProviderFormProps {
  /**
   * Save the provider. A rejection is shown in the form, which stays open with
   * everything typed (T3-SH-3: the daemon refuses a key the provider rejects).
   */
  onSubmit: (data: UpdateCustomProviderRequest) => void | Promise<void>;
  onCancel: () => void;
  initialData: UpdateCustomProviderRequest | null;
  isEditable?: boolean;
  /**
   * Whether a key is already saved for the provider being edited. Declarative
   * providers (DeepSeek, Groq, Mistral...) open here with `initialData` whether
   * or not anyone set them up, and the form used to read that as "a key is
   * saved": "Leave blank to keep existing key", "Update Provider", and an empty
   * key accepted as a silent no-op (W2-PRV-13). Defaults to `true` for an edit,
   * the old reading, so a caller that knows nothing changes nothing.
   */
  hasSavedKey?: boolean;
}

export default function CustomProviderForm({
  onSubmit,
  onCancel,
  initialData,
  isEditable,
  hasSavedKey = true,
}: CustomProviderFormProps) {
  /** An edit of a provider whose key is saved: a blank key keeps it. */
  const keepsSavedKey = initialData !== null && hasSavedKey;
  /**
   * W2-PRV-2, round 4. In a browser served by `biorouter serve`, the daemon
   * refuses to move a saved key to a new URL, since nothing there can confirm a
   * person asked. Typing the key again replaces it, and that is allowed.
   */
  const urlNeedsTypedKey = keepsSavedKey && isBrowserSurface();
  const [engine, setEngine] = useState('openai_compatible');
  const [displayName, setDisplayName] = useState('');
  const [apiUrl, setApiUrl] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [models, setModels] = useState('');
  const [isLocalModel, setIsLocalModel] = useState(false);
  const [supportsStreaming, setSupportsStreaming] = useState(true);
  const [validationErrors, setValidationErrors] = useState<Record<string, string>>({});
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (initialData) {
      const engineMap: Record<string, string> = {
        openai: 'openai_compatible',
        anthropic: 'anthropic_compatible',
        ollama: 'ollama_compatible',
      };
      setEngine(engineMap[initialData.engine] || 'openai_compatible');
      setDisplayName(initialData.display_name);
      setApiUrl(initialData.api_url);
      setModels(initialData.models.join(', '));
      setSupportsStreaming(initialData.supports_streaming ?? true);
    }
  }, [initialData]);

  const handleLocalModels = (checked: boolean) => {
    setIsLocalModel(checked);
    if (checked) {
      setApiKey('notrequired');
    } else {
      setApiKey('');
    }
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();

    const errors: Record<string, string> = {};
    if (!displayName) errors.displayName = 'Display name is required';
    if (!apiUrl) errors.apiUrl = 'API URL is required';
    if (!isLocalModel && !apiKey && !keepsSavedKey) errors.apiKey = 'API key is required';
    if (urlNeedsTypedKey && !apiKey && apiUrl !== initialData?.api_url) {
      errors.apiKey = 'Type the key again to move this provider to a new URL.';
    }
    if (!models) errors.models = 'At least one model is required';

    if (Object.keys(errors).length > 0) {
      setValidationErrors(errors);
      return;
    }

    const modelList = models
      .split(',')
      .map((m) => m.trim())
      .filter((m) => m);

    // T3-SH-3: a save can be refused (a key the provider rejected), and that
    // used to be an unhandled rejection with the form sitting there as if
    // nothing had happened. The sentence is shown here, in the form. The save
    // itself is started synchronously, as it always was.
    setSubmitError(null);
    let saved: void | Promise<void>;
    try {
      saved = onSubmit({
        engine,
        display_name: displayName,
        api_url: apiUrl,
        api_key: apiKey,
        models: modelList,
        supports_streaming: supportsStreaming,
      });
    } catch (error) {
      setSubmitError(customProviderSaveFailure(error));
      return;
    }
    if (saved instanceof Promise) {
      setSaving(true);
      saved
        .catch((error: unknown) => setSubmitError(customProviderSaveFailure(error)))
        .finally(() => setSaving(false));
    }
  };

  return (
    <form onSubmit={handleSubmit} className="mt-4 space-y-4">
      {isEditable && (
        <>
          <div>
            <label
              htmlFor="provider-select"
              className="flex items-center text-sm font-medium text-text-default mb-2"
            >
              Provider Type
              <span className="text-text-danger ml-1">*</span>
            </label>
            <Select
              id="provider-select"
              aria-invalid={!!validationErrors.providerType}
              aria-describedby={validationErrors.providerType ? 'provider-select-error' : undefined}
              options={[
                { value: 'openai_compatible', label: 'OpenAI Compatible' },
                { value: 'anthropic_compatible', label: 'Anthropic Compatible' },
                { value: 'ollama_compatible', label: 'Ollama Compatible' },
              ]}
              value={{
                value: engine,
                label:
                  engine === 'openai_compatible'
                    ? 'OpenAI Compatible'
                    : engine === 'anthropic_compatible'
                      ? 'Anthropic Compatible'
                      : 'Ollama Compatible',
              }}
              onChange={(option: unknown) => {
                const selectedOption = option as { value: string; label: string } | null;
                if (selectedOption) setEngine(selectedOption.value);
              }}
              isSearchable={false}
            />
            {validationErrors.providerType && (
              <p id="provider-select-error" className="text-text-danger text-sm mt-1">
                {validationErrors.providerType}
              </p>
            )}
          </div>
          <div>
            <label
              htmlFor="display-name"
              className="flex items-center text-sm font-medium text-text-default mb-2"
            >
              Display Name
              <span className="text-text-danger ml-1">*</span>
            </label>
            <Input
              id="display-name"
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              placeholder="Your Provider Name"
              aria-invalid={!!validationErrors.displayName}
              aria-describedby={validationErrors.displayName ? 'display-name-error' : undefined}
              className={validationErrors.displayName ? 'border-border-danger' : ''}
            />
            {validationErrors.displayName && (
              <p id="display-name-error" className="text-text-danger text-sm mt-1">
                {validationErrors.displayName}
              </p>
            )}
          </div>
          <div>
            <label
              htmlFor="api-url"
              className="flex items-center text-sm font-medium text-text-default mb-2"
            >
              API URL
              <span className="text-text-danger ml-1">*</span>
            </label>
            <Input
              id="api-url"
              value={apiUrl}
              onChange={(e) => setApiUrl(e.target.value)}
              placeholder="https://api.example.com/v1"
              aria-invalid={!!validationErrors.apiUrl}
              aria-describedby={validationErrors.apiUrl ? 'api-url-error' : undefined}
              className={validationErrors.apiUrl ? 'border-border-danger' : ''}
            />
            {validationErrors.apiUrl && (
              <p id="api-url-error" className="text-text-danger text-sm mt-1">
                {validationErrors.apiUrl}
              </p>
            )}
            {urlNeedsTypedKey && (
              <p data-testid="host-managed-custom-url" className="text-text-muted text-sm mt-1">
                {HOST_MANAGED_CUSTOM_URL}
              </p>
            )}
          </div>
        </>
      )}

      <div>
        <label
          htmlFor="api-key"
          className="flex items-center text-sm font-medium text-text-default mb-2"
        >
          API Key
          {!isLocalModel && !keepsSavedKey && <span className="text-text-danger ml-1">*</span>}
        </label>
        {/* The same primitive the built-in providers' form uses for every secret
            parameter, so the two forms mask — and reveal — a key the same way. */}
        <SecretInput
          id="api-key"
          revealLabel="API Key"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          placeholder={keepsSavedKey ? 'Leave blank to keep existing key' : 'Your API key'}
          aria-invalid={!!validationErrors.apiKey}
          aria-describedby={validationErrors.apiKey ? 'api-key-error' : undefined}
          className={validationErrors.apiKey ? 'border-border-danger' : ''}
          disabled={isLocalModel}
        />
        {validationErrors.apiKey && (
          <p id="api-key-error" className="text-text-danger text-sm mt-1">
            {validationErrors.apiKey}
          </p>
        )}

        {!initialData && (
          <div className="flex items-center space-x-2 mt-2">
            <Checkbox id="local-model" checked={isLocalModel} onCheckedChange={handleLocalModels} />
            <label
              htmlFor="local-model"
              className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70 text-text-muted"
            >
              This is a local model (no auth required)
            </label>
          </div>
        )}
      </div>
      {isEditable && (
        <>
          <div>
            <label
              htmlFor="available-models"
              className="flex items-center text-sm font-medium text-text-default mb-2"
            >
              Available Models (comma-separated)
              <span className="text-text-danger ml-1">*</span>
            </label>
            <Input
              id="available-models"
              value={models}
              onChange={(e) => setModels(e.target.value)}
              placeholder="model-a, model-b, model-c"
              aria-invalid={!!validationErrors.models}
              aria-describedby={validationErrors.models ? 'available-models-error' : undefined}
              className={validationErrors.models ? 'border-border-danger' : ''}
            />
            {validationErrors.models && (
              <p id="available-models-error" className="text-text-danger text-sm mt-1">
                {validationErrors.models}
              </p>
            )}
          </div>
          <div className="flex items-center space-x-2 mb-10">
            <Checkbox
              id="supports-streaming"
              checked={supportsStreaming}
              onCheckedChange={(checked) => setSupportsStreaming(checked as boolean)}
            />
            <label
              htmlFor="supports-streaming"
              className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70 text-text-muted"
            >
              Provider supports streaming responses
            </label>
          </div>
        </>
      )}
      <SecureStorageNotice />
      {submitError && (
        <p
          role="alert"
          data-testid="custom-provider-submit-error"
          className="text-text-danger text-sm whitespace-pre-wrap break-words"
        >
          {submitError}
        </p>
      )}
      <div className="flex justify-end space-x-2 pt-4">
        <Button type="button" variant="outline" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" disabled={saving}>
          {initialData ? (keepsSavedKey ? 'Update Provider' : 'Save') : 'Create Provider'}
        </Button>
      </div>
    </form>
  );
}
