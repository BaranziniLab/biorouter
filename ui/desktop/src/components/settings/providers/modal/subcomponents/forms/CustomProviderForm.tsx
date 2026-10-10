import React, { useState, useEffect } from 'react';
import { Input } from '../../../../../ui/input';
import { SecretInput } from '../../../../../ui/secret-input';
import { Select } from '../../../../../ui/Select';
import { Button } from '../../../../../ui/button';
import { Checkbox } from '../../../../../ui/Checkbox';
import { Field } from '../../../../../ui/field';
import { Note } from '../../../../../ui/note';
import { SecureStorageNotice } from '../SecureStorageNotice';
import { UpdateCustomProviderRequest } from '../../../../../../api';
import { isBrowserSurface } from '../../../../../../utils/surface';
import { HOST_MANAGED_CUSTOM_URL } from '../../../../../privacy/hostManagedModelCopy';
import { customProviderCopy } from '../../../copy';

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

  const engineLabel = (value: string) =>
    value === 'openai_compatible'
      ? 'OpenAI compatible'
      : value === 'anthropic_compatible'
        ? 'Anthropic compatible'
        : 'Ollama compatible';

  return (
    <form onSubmit={handleSubmit} className="flex flex-col gap-4 py-3">
      {isEditable && (
        <>
          <Field
            id="provider-select"
            label={customProviderCopy.providerType}
            error={validationErrors.providerType}
          >
            {/* react-select names its own input: the label points at `inputId`, and the
                container keeps `id` so Field's helper id stays unique. */}
            <Select
              inputId="provider-select-input"
              aria-label={customProviderCopy.providerType}
              options={[
                { value: 'openai_compatible', label: engineLabel('openai_compatible') },
                { value: 'anthropic_compatible', label: engineLabel('anthropic_compatible') },
                { value: 'ollama_compatible', label: engineLabel('ollama_compatible') },
              ]}
              value={{ value: engine, label: engineLabel(engine) }}
              onChange={(option: unknown) => {
                const selectedOption = option as { value: string; label: string } | null;
                if (selectedOption) setEngine(selectedOption.value);
              }}
              isSearchable={false}
              menuPlacement="auto"
              menuPortalTarget={typeof document === 'undefined' ? undefined : document.body}
            />
          </Field>
          <Field
            id="display-name"
            label={customProviderCopy.displayName}
            error={validationErrors.displayName}
          >
            <Input
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              placeholder="Your provider name"
            />
          </Field>
          <Field
            id="api-url"
            label={customProviderCopy.apiUrl}
            error={validationErrors.apiUrl}
            helper={
              urlNeedsTypedKey ? (
                <span data-testid="host-managed-custom-url">{HOST_MANAGED_CUSTOM_URL}</span>
              ) : undefined
            }
          >
            <Input
              value={apiUrl}
              onChange={(e) => setApiUrl(e.target.value)}
              placeholder="https://api.example.com/v1"
            />
          </Field>
        </>
      )}

      <div className="flex flex-col gap-2">
        <Field id="api-key" label={customProviderCopy.apiKey} error={validationErrors.apiKey}>
          {/* The same primitive the built-in providers' form uses for every secret
              parameter, so the two forms mask — and reveal — a key the same way. */}
          <SecretInput
            revealLabel={customProviderCopy.apiKey}
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder={keepsSavedKey ? 'Leave blank to keep existing key' : 'Your API key'}
            disabled={isLocalModel}
          />
        </Field>
        {!initialData && (
          <label className="flex cursor-pointer items-center gap-2">
            <Checkbox
              id="local-model"
              checked={isLocalModel}
              onChange={(event) => handleLocalModels(event.target.checked)}
            />
            <span className="text-label text-text-default">{customProviderCopy.localModel}</span>
          </label>
        )}
      </div>
      {isEditable && (
        <>
          <Field
            id="available-models"
            label={customProviderCopy.models}
            error={validationErrors.models}
          >
            <Input
              value={models}
              onChange={(e) => setModels(e.target.value)}
              placeholder="model-a, model-b, model-c"
            />
          </Field>
          <label className="flex cursor-pointer items-center gap-2">
            <Checkbox
              id="supports-streaming"
              checked={supportsStreaming}
              onChange={(event) => setSupportsStreaming(event.target.checked)}
            />
            <span className="text-label text-text-default">{customProviderCopy.streaming}</span>
          </label>
        </>
      )}
      <SecureStorageNotice className="mt-0" />
      {submitError && (
        <Note tone="danger" role="alert" testId="custom-provider-submit-error">
          <span className="whitespace-pre-wrap break-words">{submitError}</span>
        </Note>
      )}
      <div className="flex justify-end gap-2 pt-1">
        <Button type="button" variant="secondary" onClick={onCancel}>
          {customProviderCopy.cancel}
        </Button>
        <Button type="submit" disabled={saving}>
          {initialData
            ? keepsSavedKey
              ? customProviderCopy.update
              : customProviderCopy.save
            : customProviderCopy.create}
        </Button>
      </div>
    </form>
  );
}
