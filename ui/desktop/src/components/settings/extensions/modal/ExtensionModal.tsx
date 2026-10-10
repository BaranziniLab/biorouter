import { useState, useCallback } from 'react';
import { Button } from '../../../ui/button';
import { ModalShell } from '../../../ModalShell';
import { InfoTip } from '../../../ui/info-tip';
import { Note } from '../../../ui/note';
import { ExtensionFormData } from '../utils';
import EnvVarsSection from './EnvVarsSection';
import HeadersSection from './HeadersSection';
import ExtensionConfigFields from './ExtensionConfigFields';
import { Info } from '../../../icons/app-icons';
import ExtensionInfoFields from './ExtensionInfoFields';
import ExtensionTimeoutField from './ExtensionTimeoutField';
import { upsertConfig } from '../../../../api';
import { userActionHeaders } from '../../../../utils/userAction';
import { ConfirmationModal } from '../../../ui/ConfirmationModal';
import { PrivacyBadge } from '../../../ui/PrivacyBadge';
import { classifyExtension } from '../extensionPrivacy';
import ExtensionCredentialsDialog from './ExtensionCredentialsDialog';
import { EXTENSION_MODAL_COPY as COPY } from '../copy';
import './extension-modal.css';

interface ExtensionModalProps {
  title: string;
  initialData: ExtensionFormData;
  onClose: () => void;
  onSubmit: (formData: ExtensionFormData) => void;
  onDelete?: (name: string) => void;
  submitLabel: string;
  modalType: 'add' | 'edit';
  /**
   * Open at the removal step: the row's Remove… item asks to remove, not to edit, so the
   * dialog starts on the question and Cancel closes it rather than showing a form.
   */
  startAtRemoval?: boolean;
}

export default function ExtensionModal({
  title,
  initialData,
  onClose,
  onSubmit,
  onDelete,
  submitLabel,
  modalType,
  startAtRemoval = false,
}: ExtensionModalProps) {
  const [formData, setFormData] = useState<ExtensionFormData>(initialData);
  const [showDeleteConfirmation, setShowDeleteConfirmation] = useState(startAtRemoval);
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

  // Update title based on current state. No title icon: no other dialog has one.
  const modalTitle = showDeleteConfirmation ? COPY.removeTitle(formData.name) : title;

  // Cancel at the removal step goes back to the form, unless the person never saw the form.
  const cancelRemoval = () => (startAtRemoval ? onClose() : setShowDeleteConfirmation(false));

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
  const tierNotice = resultingTier === 'private' ? COPY.tierPrivate : COPY.tierPublic;

  return (
    <>
      <ModalShell
        open
        onOpenChange={(open) => {
          if (!open) handleClose();
        }}
        size="lg"
        // A form: Escape and the × still leave (through the unsaved-changes guard), but a
        // misclick on the scrim cannot throw away what was typed.
        purpose="form"
        // The form grows and shrinks as the type changes and rows are added; anchored at the
        // top it grows downward, so the fields under the pointer do not move.
        anchor="top"
        scrollBody
        title={modalTitle}
        // The consequence of removing is the one description the dialog has, and only while it
        // is being asked. Without it the dialog carries no description at all.
        subtitle={showDeleteConfirmation ? COPY.removeDescription : undefined}
        bodyClassName="py-4"
        footer={
          showDeleteConfirmation ? (
            <>
              {/* First focus lands on the safe answer, not on "Review saved credentials" and not
                  on the removal button: Radix keeps a focus that is already inside the dialog. */}
              <Button variant="secondary" onClick={cancelRemoval} autoFocus>
                {COPY.cancel}
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
                {COPY.confirmRemoval}
              </Button>
            </>
          ) : (
            <>
              {modalType === 'edit' && onDelete && !isBuiltin && (
                <Button
                  onClick={() => setShowDeleteConfirmation(true)}
                  variant="destructive"
                  className="mr-auto"
                >
                  {COPY.removeAction}
                </Button>
              )}
              <Button variant="secondary" onClick={handleClose}>
                {COPY.cancel}
              </Button>
              <Button
                data-testid="extension-submit-btn"
                onClick={handleSubmit}
                disabled={!isFormValid()}
              >
                {submitLabel}
              </Button>
            </>
          )
        }
      >
        {showDeleteConfirmation ? (
          <div className="flex items-center gap-1">
            <Button variant="secondary" size="sm" onClick={() => setShowCredentials(true)}>
              {COPY.reviewCredentials}
            </Button>
            <InfoTip label={COPY.reviewCredentials} help={COPY.sharedCredentialsHelp} />
          </div>
        ) : (
          <div className="br-ext-form">
            {formData.installation_notes && (
              <Note tone="info" icon={Info}>
                <div className="text-label text-text-default">{COPY.installationNotes}</div>
                <div className="mt-1">{formData.installation_notes}</div>
              </Note>
            )}

            {isBuiltin ? (
              <>
                {/* A built-in's name and description are facts, not fields: Crew's label-over-value
                    rows, and only the timeout can be changed. */}
                <dl className="br-ext-facts">
                  <div>
                    <dt className="text-supporting text-text-muted">{COPY.nameLabel}</dt>
                    <dd className="text-label text-text-default">{formData.name}</dd>
                  </div>
                  {formData.description && (
                    <div>
                      <dt className="text-supporting text-text-muted">{COPY.descriptionLabel}</dt>
                      <dd className="text-body text-text-default">{formData.description}</dd>
                    </div>
                  )}
                </dl>

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
                  // §13.5, and it stays on screen: the tier this name will produce, said before
                  // the person commits. A badge and the one sentence, no box around either.
                  <div className="br-ext-tier">
                    <PrivacyBadge tier={resultingTier} />
                    <p className="text-supporting text-text-muted">{tierNotice}</p>
                  </div>
                )}

                <ExtensionConfigFields
                  type={formData.type}
                  full_cmd={formData.cmd || ''}
                  endpoint={formData.endpoint || ''}
                  onChange={(key, value) => setFormData({ ...formData, [key]: value })}
                  submitAttempted={submitAttempted}
                  isValid={isConfigValid()}
                />
                <ExtensionTimeoutField
                  timeout={formData.timeout || 300}
                  onChange={(key, value) => setFormData({ ...formData, [key]: value })}
                  submitAttempted={submitAttempted}
                />
              </>
            )}

            {!isBuiltin && formData.type === 'stdio' && (
              <EnvVarsSection
                envVars={formData.envVars}
                onAdd={handleAddEnvVar}
                onRemove={handleRemoveEnvVar}
                onChange={handleEnvVarChange}
                submitAttempted={submitAttempted}
                onPendingInputChange={setHasPendingEnvVars}
              />
            )}

            {!isBuiltin && formData.type === 'streamable_http' && (
              <HeadersSection
                headers={formData.headers}
                onAdd={handleAddHeader}
                onRemove={handleRemoveHeader}
                onChange={handleHeaderChange}
                submitAttempted={submitAttempted}
                onPendingInputChange={handlePendingHeaderChange}
              />
            )}

            {modalType === 'edit' && !isBuiltin && (
              // Removing keeps credentials; reviewing them is its own step, one button and the
              // sentence behind an InfoTip, at the foot of the form rather than above it.
              <div className="flex items-center gap-1">
                <Button variant="secondary" size="sm" onClick={() => setShowCredentials(true)}>
                  {COPY.reviewCredentials}
                </Button>
                <InfoTip label={COPY.reviewCredentials} help={COPY.credentialsHelp} />
              </div>
            )}
          </div>
        )}
      </ModalShell>

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
          title={COPY.unsavedTitle}
          message={COPY.unsavedMessage}
          confirmLabel={COPY.discard}
          cancelLabel={COPY.cancel}
          onConfirm={handleConfirmClose}
          onCancel={handleCancelClose}
        />
      )}
    </>
  );
}
