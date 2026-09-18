import { useState, useCallback } from 'react';
import { Button } from '../../../ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../../ui/dialog';
import { ExtensionFormData } from '../utils';
import EnvVarsSection from './EnvVarsSection';
import HeadersSection from './HeadersSection';
import ExtensionConfigFields from './ExtensionConfigFields';
import { PlusIcon, Edit, Trash2, AlertTriangle, Info } from '../../../icons/app-icons';
import ExtensionInfoFields from './ExtensionInfoFields';
import ExtensionTimeoutField from './ExtensionTimeoutField';
import { upsertConfig } from '../../../../api';
import { userActionHeaders } from '../../../../utils/userAction';
import { ConfirmationModal } from '../../../ui/ConfirmationModal';
import { PrivacyBadge } from '../../../ui/PrivacyBadge';
import { classifyExtension } from '../extensionPrivacy';
import ExtensionCredentialsDialog from './ExtensionCredentialsDialog';

interface ExtensionModalProps {
  title: string;
  initialData: ExtensionFormData;
  onClose: () => void;
  onSubmit: (formData: ExtensionFormData) => void;
  onDelete?: (name: string) => void;
  submitLabel: string;
  modalType: 'add' | 'edit';
}

export default function ExtensionModal({
  title,
  initialData,
  onClose,
  onSubmit,
  onDelete,
  submitLabel,
  modalType,
}: ExtensionModalProps) {
  const [formData, setFormData] = useState<ExtensionFormData>(initialData);
  const [showDeleteConfirmation, setShowDeleteConfirmation] = useState(false);
  const [showCredentials, setShowCredentials] = useState(false);
  const [submitAttempted, setSubmitAttempted] = useState(false);
  const [showCloseConfirmation, setShowCloseConfirmation] = useState(false);
  const [hasPendingEnvVars, setHasPendingEnvVars] = useState(false);
  const [hasPendingHeaders, setHasPendingHeaders] = useState(false);
  const [pendingHeader, setPendingHeader] = useState<{ key: string; value: string } | null>(null);

  const isBuiltin = formData.type === 'builtin';

  // Function to check if form has been modified
  const hasFormChanges = (): boolean => {
    if (isBuiltin) {
      return formData.timeout !== initialData.timeout;
    }

    // Check if command/endpoint has changed
    const commandChanged =
      (formData.type === 'stdio' && formData.cmd !== initialData.cmd) ||
      (formData.type === 'sse' && formData.endpoint !== initialData.endpoint) ||
      (formData.type === 'streamable_http' && formData.endpoint !== initialData.endpoint);

    // Check if headers have changed
    const headersChanged = formData.headers.some((header) => header.isEdited === true);

    // Check if any environment variables have been modified
    const envVarsChanged = formData.envVars.some((envVar) => envVar.isEdited === true);

    // Check if new env vars have been added
    const envVarsAdded = formData.envVars.length > initialData.envVars.length;

    // Check if env vars have been removed
    const envVarsRemoved = formData.envVars.length < initialData.envVars.length;

    // Check if any environment variable fields have text entered (even if not marked as edited)
    const envVarsHaveText = formData.envVars.some(
      (envVar) =>
        (envVar.key.trim() !== '' || envVar.value.trim() !== '') &&
        // Don't count placeholder values for existing secrets
        envVar.value !== '••••••••'
    );

    // Check if there are pending environment variables or headers being typed
    const hasPendingInput = hasPendingEnvVars || hasPendingHeaders;

    return (
      commandChanged ||
      headersChanged ||
      envVarsChanged ||
      envVarsAdded ||
      envVarsRemoved ||
      envVarsHaveText ||
      hasPendingInput
    );
  };

  // Handle backdrop close with confirmation if needed
  const handleClose = () => {
    if (hasFormChanges()) {
      setShowCloseConfirmation(true);
    } else {
      onClose();
    }
  };

  // Handle confirmed close
  const handleConfirmClose = () => {
    setShowCloseConfirmation(false);
    onClose();
  };

  // Handle cancel close confirmation
  const handleCancelClose = () => {
    setShowCloseConfirmation(false);
  };

  const handleAddEnvVar = (key: string, value: string) => {
    setFormData({
      ...formData,
      envVars: [...formData.envVars, { key, value, isEdited: true }],
    });
  };

  const handleRemoveEnvVar = (index: number) => {
    const newEnvVars = [...formData.envVars];
    newEnvVars.splice(index, 1);
    setFormData({
      ...formData,
      envVars: newEnvVars,
    });
  };

  const handleEnvVarChange = (index: number, field: 'key' | 'value', value: string) => {
    const newEnvVars = [...formData.envVars];
    newEnvVars[index][field] = value;

    // Mark as edited if it's a value change
    if (field === 'value') {
      newEnvVars[index].isEdited = true;
    }

    setFormData({
      ...formData,
      envVars: newEnvVars,
    });
  };

  const handleAddHeader = (key: string, value: string) => {
    setFormData({
      ...formData,
      headers: [...formData.headers, { key, value, isEdited: true }],
    });
  };

  const handleRemoveHeader = (index: number) => {
    const newHeaders = [...formData.headers];
    newHeaders.splice(index, 1);
    setFormData({
      ...formData,
      headers: newHeaders,
    });
  };

  const handleHeaderChange = (index: number, field: 'key' | 'value', value: string) => {
    const newHeaders = [...formData.headers];
    newHeaders[index][field] = value;

    // Mark as edited if it's a value change
    if (field === 'value') {
      newHeaders[index].isEdited = true;
    }

    setFormData({
      ...formData,
      headers: newHeaders,
    });
  };

  const handlePendingHeaderChange = useCallback(
    (hasPending: boolean, header: { key: string; value: string } | null) => {
      setHasPendingHeaders(hasPending);
      setPendingHeader(header);
    },
    []
  );

  // Function to store a secret value
  const storeSecret = async (key: string, value: string) => {
    try {
      await upsertConfig({
        body: {
          is_secret: true,
          key: key,
          value: value,
        },
        // Issue #56 DR-16: same reason as `BrxtInstallModal` — the key is the
        // extension author's, so it can collide with a capability key, and the
        // guard does not look at `is_secret`. A person saving an extension's
        // settings is a user act and carries the proof.
        headers: await userActionHeaders(),
      });
      return true;
    } catch (error) {
      console.error('Failed to store secret:', error);
      return false;
    }
  };

  // Function to determine which icon to display with proper styling
  const getModalIcon = () => {
    if (showDeleteConfirmation) {
      return <AlertTriangle className="text-text-danger" size={24} />;
    }
    return modalType === 'add' ? (
      <PlusIcon className="text-iconStandard" size={24} />
    ) : (
      <Edit className="text-iconStandard" size={24} />
    );
  };

  const isNameValid = () => {
    return formData.name.trim() !== '';
  };

  const isConfigValid = () => {
    return (
      isBuiltin ||
      (formData.type === 'stdio' && !!formData.cmd && formData.cmd.trim() !== '') ||
      (formData.type === 'sse' && !!formData.endpoint && formData.endpoint.trim() !== '') ||
      (formData.type === 'streamable_http' &&
        !!formData.endpoint &&
        formData.endpoint.trim() !== '')
    );
  };

  const isEnvVarsValid = () => {
    return formData.envVars.every(
      ({ key, value }) => (key === '' && value === '') || (key !== '' && value !== '')
    );
  };

  const getFinalHeaders = () => {
    const finalHeaders = [...formData.headers];
    if (pendingHeader && pendingHeader.key.trim() !== '' && pendingHeader.value.trim() !== '') {
      finalHeaders.push({ ...pendingHeader, isEdited: true });
    }
    return finalHeaders;
  };

  const isHeadersValid = () => {
    return getFinalHeaders().every(
      ({ key, value }) => (key === '' && value === '') || (key !== '' && value !== '')
    );
  };

  const isTimeoutValid = () => {
    // Check if timeout is not undefined, null, or empty string
    if (formData.timeout === undefined || formData.timeout === null) {
      return false;
    }

    // Convert to number if it's a string
    const timeoutValue =
      typeof formData.timeout === 'string' ? Number(formData.timeout) : formData.timeout;

    // Check if it's a valid number (not NaN) and is a positive number
    return !isNaN(timeoutValue) && timeoutValue > 0;
  };

  // Form validation
  const isFormValid = () => {
    return (
      isNameValid() && isConfigValid() && isEnvVarsValid() && isHeadersValid() && isTimeoutValid()
    );
  };

  // Handle submit with validation and secret storage
  const handleSubmit = async () => {
    setSubmitAttempted(true);

    if (isFormValid()) {
      const finalFormData = {
        ...formData,
        headers: getFinalHeaders(),
      };

      // Only store env vars that have been edited (which includes new)
      const secretPromises = finalFormData.envVars
        .filter((envVar) => envVar.isEdited)
        .map(({ key, value }) => storeSecret(key, value));

      try {
        // Wait for all secrets to be stored
        const results = await Promise.all(secretPromises);

        if (results.every((success) => success)) {
          // Convert timeout to number if needed
          const dataToSubmit = {
            ...finalFormData,
            timeout:
              typeof finalFormData.timeout === 'string'
                ? Number(finalFormData.timeout)
                : finalFormData.timeout,
          };
          onSubmit(dataToSubmit);
          onClose();
        } else {
          console.error('Failed to store one or more secrets');
        }
      } catch (error) {
        console.error('Error during submission:', error);
      }
    } else {
      console.log('Form validation failed');
    }
  };

  // Update title based on current state
  const modalTitle = showDeleteConfirmation ? `Delete Extension "${formData.name}"` : title;

  /**
   * Issue #56 §13.5: "The manual 'Add stdio extension' form carries the same
   * line." This is the one install route with no bundle and no catalogue entry
   * behind it, so nothing else on the screen hints at what the result will be.
   *
   * It tracks the NAME as it is typed, because on this form the name is the
   * entire input to the tier — `classify_extension` keys on
   * `name_to_key(name)` and nothing else. That makes DR-19's consequence
   * visible at the one moment a person can trigger it through the GUI: a
   * published private name produces a Private extension, any other spelling of
   * it does not. It discloses; it does not restrict. Reserving names here would
   * be the wrong repair (open question 28 wants provenance recorded at install,
   * not a blocklist on a text field), and it would not stop the agent-writable
   * `config.yaml` path this form is only one entrance to.
   *
   * Add only. On an edit the extension already exists and the two "installed
   * how" sentences would be guesses — a marketplace install is edited through
   * this same modal.
   */
  const resultingTier = classifyExtension(formData.name);
  const tierNotice =
    resultingTier === 'private'
      ? 'The Biorouter marketplace publishes this name as private, so this extension will be Private: only private models will be able to call it.'
      : 'Extensions you add by hand are always Public. Any model, including commercial models hosted outside your institution, will be able to call this extension.';

  return (
    <>
      <Dialog open={true} onOpenChange={handleClose}>
        <DialogContent
          {...(showDeleteConfirmation ? {} : { 'aria-describedby': undefined })}
          className="sm:max-w-[600px] max-h-[90vh] overflow-y-auto"
        >
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              {getModalIcon()}
              {modalTitle}
            </DialogTitle>
            {showDeleteConfirmation && (
              <DialogDescription>
                This will remove the extension configuration. Saved credentials are retained and may
                be reused automatically if you reinstall.
              </DialogDescription>
            )}
          </DialogHeader>

          {showDeleteConfirmation ? (
            <div className="py-4">
              <p className="text-text-default">
                To delete saved credentials first, choose Review saved credentials. Credentials
                shared with other extensions or providers are protected.
              </p>
              <Button variant="outline" className="mt-4" onClick={() => setShowCredentials(true)}>
                Review saved credentials
              </Button>
            </div>
          ) : (
            <div className="py-4 space-y-6">
              {modalType === 'edit' && !isBuiltin && (
                <div className="space-y-2">
                  <p className="text-sm text-text-muted">
                    Removing this extension retains saved credentials for reuse. You can review and
                    delete unshared saved credentials separately.
                  </p>
                  <Button variant="outline" onClick={() => setShowCredentials(true)}>
                    Review saved credentials
                  </Button>
                </div>
              )}
              {formData.installation_notes && (
                <div className="biorouter-modal-panel rounded-container p-4">
                  <div className="flex items-start gap-2">
                    <Info className="h-5 w-5 text-text-info shrink-0 mt-0.5" />
                    <div>
                      <h4 className="text-sm font-medium text-text-default mb-1">
                        Installation Notes
                      </h4>
                      <p className="text-sm text-text-muted">{formData.installation_notes}</p>
                    </div>
                  </div>
                </div>
              )}

              {/* Form Fields */}
              {isBuiltin ? (
                <>
                  <div className="space-y-2">
                    <div>
                      <label className="text-sm font-medium text-text-default">Name</label>
                      <p className="text-sm text-text-muted mt-1">{formData.name}</p>
                    </div>
                    {formData.description && (
                      <div>
                        <label className="text-sm font-medium text-text-default">Description</label>
                        <p className="text-sm text-text-muted mt-1">{formData.description}</p>
                      </div>
                    )}
                  </div>

                  <div className="h-px shadow-[inset_0_1px_0_color-mix(in_srgb,var(--border-subtle)_45%,transparent)]" />

                  <ExtensionTimeoutField
                    timeout={formData.timeout || 300}
                    onChange={(key, value) => setFormData({ ...formData, [key]: value })}
                    submitAttempted={submitAttempted}
                  />
                </>
              ) : (
                <>
                  {/* Name and Type */}
                  <ExtensionInfoFields
                    name={formData.name}
                    type={formData.type}
                    description={formData.description}
                    onChange={(key, value) => setFormData({ ...formData, [key]: value })}
                    submitAttempted={submitAttempted}
                  />

                  {modalType === 'add' && (
                    <div className="biorouter-modal-panel rounded-lg p-3">
                      <PrivacyBadge tier={resultingTier} />
                      <p className="text-xs text-text-muted mt-1.5 leading-relaxed">{tierNotice}</p>
                    </div>
                  )}

                  <div className="h-px shadow-[inset_0_1px_0_color-mix(in_srgb,var(--border-subtle)_45%,transparent)]" />

                  {/* Command */}
                  <div>
                    <ExtensionConfigFields
                      type={formData.type}
                      full_cmd={formData.cmd || ''}
                      endpoint={formData.endpoint || ''}
                      onChange={(key, value) => setFormData({ ...formData, [key]: value })}
                      submitAttempted={submitAttempted}
                      isValid={isConfigValid()}
                    />
                    <div className="mb-4" />
                    <ExtensionTimeoutField
                      timeout={formData.timeout || 300}
                      onChange={(key, value) => setFormData({ ...formData, [key]: value })}
                      submitAttempted={submitAttempted}
                    />
                  </div>
                </>
              )}

              {!isBuiltin && formData.type === 'stdio' && (
                <>
                  <div className="h-px shadow-[inset_0_1px_0_color-mix(in_srgb,var(--border-subtle)_45%,transparent)]" />

                  <div>
                    <EnvVarsSection
                      envVars={formData.envVars}
                      onAdd={handleAddEnvVar}
                      onRemove={handleRemoveEnvVar}
                      onChange={handleEnvVarChange}
                      submitAttempted={submitAttempted}
                      onPendingInputChange={setHasPendingEnvVars}
                    />
                  </div>
                </>
              )}

              {!isBuiltin && formData.type === 'streamable_http' && (
                <>
                  {/* Divider */}
                  <div className="h-px shadow-[inset_0_1px_0_color-mix(in_srgb,var(--border-subtle)_45%,transparent)]" />

                  <div>
                    <HeadersSection
                      headers={formData.headers}
                      onAdd={handleAddHeader}
                      onRemove={handleRemoveHeader}
                      onChange={handleHeaderChange}
                      submitAttempted={submitAttempted}
                      onPendingInputChange={handlePendingHeaderChange}
                    />
                  </div>
                </>
              )}
            </div>
          )}

          <DialogFooter className="pt-2">
            {showDeleteConfirmation ? (
              <>
                <Button variant="outline" onClick={() => setShowDeleteConfirmation(false)}>
                  Cancel
                </Button>
                <Button
                  onClick={() => {
                    if (onDelete) {
                      onDelete(formData.name);
                      onClose();
                    }
                  }}
                  variant="destructive"
                >
                  <Trash2 className="h-4 w-4 mr-2" />
                  Confirm removal
                </Button>
              </>
            ) : (
              <>
                {modalType === 'edit' && onDelete && !isBuiltin && (
                  <Button
                    onClick={() => setShowDeleteConfirmation(true)}
                    variant="outline"
                    className="text-text-danger hover:bg-background-danger/10"
                  >
                    <Trash2 className="h-4 w-4 mr-2" />
                    Remove extension
                  </Button>
                )}
                <Button variant="outline" onClick={handleClose}>
                  Cancel
                </Button>
                <Button
                  data-testid="extension-submit-btn"
                  onClick={handleSubmit}
                  disabled={!isFormValid()}
                >
                  {submitLabel}
                </Button>
              </>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {showCredentials && (
        <ExtensionCredentialsDialog
          name={initialData.name}
          onClose={() => setShowCredentials(false)}
          onDeleted={(keys) => {
            setFormData((current) => ({
              ...current,
              envVars: current.envVars.map((envVar) =>
                keys.includes(envVar.key) && !envVar.isEdited
                  ? { ...envVar, value: '', isEdited: false }
                  : envVar
              ),
            }));
          }}
        />
      )}

      {/* Close Confirmation Modal */}
      {showCloseConfirmation && (
        <ConfirmationModal
          isOpen={showCloseConfirmation}
          title="Unsaved Changes"
          message="You have unsaved changes to the extension configuration. Are you sure you want to close without saving?"
          confirmLabel="Close Without Saving"
          onConfirm={handleConfirmClose}
          onCancel={handleCancelClose}
        />
      )}
    </>
  );
}
