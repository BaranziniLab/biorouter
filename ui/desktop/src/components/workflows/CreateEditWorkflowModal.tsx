import React, { useState, useEffect, useCallback } from 'react';
import { useForm } from '@tanstack/react-form';
import { Workflow, generateDeepLink, Parameter } from '../../workflow';
import { Check, Copy, ExternalLink, Play, Save } from '../icons/app-icons';
import { ExtensionConfig } from '../ConfigContext';
import { Button } from '../ui/button';
import { Input } from '../ui/input';

import { WorkflowFormFields } from './shared/WorkflowFormFields';
import { WorkflowFormData } from './shared/workflowFormSchema';
import { toastSuccess, toastError } from '../../toasts';
import { saveWorkflow } from '../../workflow/workflow_management';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '../ui/dialog';
import { storeSubscriptionCleanup } from '../../utils/storeSubscription';

interface CreateEditWorkflowModalProps {
  isOpen: boolean;
  onClose: (wasSaved?: boolean) => void;
  workflow?: Workflow;
  isCreateMode?: boolean;
  workflowId?: string | null;
}

export default function CreateEditWorkflowModal({
  isOpen,
  onClose,
  workflow,
  isCreateMode = false,
  workflowId,
}: CreateEditWorkflowModalProps) {
  const getInitialValues = React.useCallback((): WorkflowFormData => {
    if (workflow) {
      return {
        title: workflow.title || '',
        description: workflow.description || '',
        instructions: workflow.instructions || '',
        prompt: workflow.prompt || '',
        activities: workflow.activities || [],
        parameters: workflow.parameters || [],
        jsonSchema: workflow.response?.json_schema
          ? JSON.stringify(workflow.response.json_schema, null, 2)
          : '',
        settings: workflow.settings
          ? {
              biorouter_provider: workflow.settings.biorouter_provider ?? undefined,
              biorouter_model: workflow.settings.biorouter_model ?? undefined,
              temperature: workflow.settings.temperature ?? undefined,
            }
          : undefined,
      };
    }
    return {
      title: '',
      description: '',
      instructions: '',
      prompt: '',
      activities: [],
      parameters: [],
      jsonSchema: '',
      settings: undefined,
    };
  }, [workflow]);

  const form = useForm({
    defaultValues: getInitialValues(),
  });

  // Helper functions to get values from form - using state to trigger re-renders
  const [title, setTitle] = useState(form.state.values.title);
  const [description, setDescription] = useState(form.state.values.description);
  const [instructions, setInstructions] = useState(form.state.values.instructions);
  const [prompt, setPrompt] = useState(form.state.values.prompt);
  const [activities, setActivities] = useState(form.state.values.activities);
  const [parameters, setParameters] = useState(form.state.values.parameters);
  const [jsonSchema, setJsonSchema] = useState(form.state.values.jsonSchema);
  const [settings, setSettings] = useState(form.state.values.settings);

  // Subscribe to form changes to update local state
  useEffect(() => {
    const subscription = form.store.subscribe(() => {
      setTitle(form.state.values.title);
      setDescription(form.state.values.description);
      setInstructions(form.state.values.instructions);
      setPrompt(form.state.values.prompt);
      setActivities(form.state.values.activities);
      setParameters(form.state.values.parameters);
      setJsonSchema(form.state.values.jsonSchema);
      setSettings(form.state.values.settings);
    });
    return storeSubscriptionCleanup(subscription);
  }, [form]);
  const [copied, setCopied] = useState(false);
  const [isSaving, setIsSaving] = useState(false);

  // Extensions for the workflow (editable)
  const [workflowExtensions, setWorkflowExtensions] = useState<ExtensionConfig[]>(
    () => workflow?.extensions ?? []
  );

  // Reset form when workflow changes
  useEffect(() => {
    if (workflow) {
      const newValues = getInitialValues();
      form.reset(newValues);
    }
  }, [workflow, form, getInitialValues]);

  const getCurrentWorkflow = useCallback((): Workflow => {
    // Transform the internal parameters state into the desired output format.
    const formattedParameters = parameters.map((param) => {
      const formattedParam: Parameter = {
        key: param.key,
        input_type: param.input_type || 'string',
        requirement: param.requirement,
        description: param.description,
      };

      // Add the 'default' key ONLY if the parameter is optional and has a default value.
      if (param.requirement === 'optional' && param.default) {
        formattedParam.default = param.default;
      }

      // Add options for select input type
      if (param.input_type === 'select' && param.options) {
        formattedParam.options = param.options.filter((opt) => opt.trim() !== ''); // Filter empty options when saving
      }

      return formattedParam;
    });

    // Parse response schema if provided
    let responseConfig = undefined;
    if (jsonSchema && jsonSchema.trim()) {
      try {
        const parsedSchema = JSON.parse(jsonSchema);
        responseConfig = { json_schema: parsedSchema };
      } catch (error) {
        console.warn('Invalid JSON schema provided:', error);
        // If JSON is invalid, don't include response config
      }
    }

    // Build settings — only include if at least one value is set
    const settingsConfig =
      settings?.biorouter_provider ||
      settings?.biorouter_model ||
      settings?.temperature !== undefined
        ? {
            biorouter_provider: settings?.biorouter_provider || undefined,
            biorouter_model: settings?.biorouter_model || undefined,
            temperature: settings?.temperature,
          }
        : undefined;

    return {
      ...workflow,
      title,
      description,
      instructions,
      activities,
      prompt: prompt || undefined,
      parameters: formattedParameters,
      response: responseConfig,
      settings: settingsConfig,
      // Strip envs to avoid leaking secrets
      extensions:
        workflowExtensions.length > 0
          ? (workflowExtensions.map((extension) =>
              'envs' in extension ? { ...extension, envs: undefined } : extension
            ) as ExtensionConfig[])
          : undefined,
    };
  }, [
    workflow,
    title,
    description,
    instructions,
    activities,
    prompt,
    parameters,
    jsonSchema,
    settings,
    workflowExtensions,
  ]);

  const requiredFieldsAreFilled = () => {
    return title.trim() && description.trim() && (instructions.trim() || (prompt || '').trim());
  };

  const validateForm = () => {
    const basicValidation =
      title.trim() && description.trim() && (instructions.trim() || (prompt || '').trim());

    // If JSON schema is provided, it must be valid
    if (jsonSchema && jsonSchema.trim()) {
      try {
        JSON.parse(jsonSchema);
      } catch {
        return false; // Invalid JSON schema fails validation
      }
    }

    return basicValidation;
  };

  const [deeplink, setDeeplink] = useState('');
  const [isGeneratingDeeplink, setIsGeneratingDeeplink] = useState(false);
  const [deeplinkError, setDeeplinkError] = useState(false);

  // Generate deeplink whenever workflow configuration changes
  useEffect(() => {
    let isCancelled = false;

    const generateLink = async () => {
      if (
        !title.trim() ||
        !description.trim() ||
        (!instructions.trim() && !(prompt || '').trim())
      ) {
        setDeeplink('');
        setDeeplinkError(false);
        return;
      }

      setIsGeneratingDeeplink(true);
      try {
        const currentWorkflow = getCurrentWorkflow();
        const link = await generateDeepLink(currentWorkflow);
        if (!isCancelled) {
          setDeeplink(link);
          setDeeplinkError(false);
        }
      } catch (error) {
        console.error('Failed to generate deeplink:', error);
        if (!isCancelled) {
          setDeeplink('');
          setDeeplinkError(true);
        }
      } finally {
        if (!isCancelled) {
          setIsGeneratingDeeplink(false);
        }
      }
    };

    generateLink();

    return () => {
      isCancelled = true;
    };
  }, [
    title,
    description,
    instructions,
    prompt,
    activities,
    parameters,
    jsonSchema,
    settings,
    workflowExtensions,
    getCurrentWorkflow,
  ]);

  const canCopyDeeplink = Boolean(deeplink) && !isGeneratingDeeplink && !deeplinkError;

  const handleCopy = () => {
    if (!canCopyDeeplink) {
      return;
    }

    navigator.clipboard
      .writeText(deeplink)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      })
      .catch((err) => {
        console.error('Failed to copy the text:', err);
      });
  };

  const handleSaveWorkflowClick = async () => {
    if (isSaving) return;
    if (!validateForm()) {
      toastError({
        title: 'Validation failed',
        msg: 'Fill in the required fields and fix the JSON schema.',
      });
      return;
    }

    setIsSaving(true);
    try {
      const workflow = getCurrentWorkflow();

      await saveWorkflow(workflow, workflowId);

      onClose(true);

      toastSuccess({
        title: (workflow.title || '').trim(),
        msg: 'Workflow saved',
      });
    } catch (error) {
      console.error('Failed to save workflow:', error);

      toastError({
        title: 'Save failed',
        msg: `Failed to save workflow: ${error instanceof Error ? error.message : 'Unknown error'}`,
        traceback: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setIsSaving(false);
    }
  };

  const handleSaveAndRunWorkflowClick = async () => {
    if (isSaving) return;
    if (!validateForm()) {
      toastError({
        title: 'Validation failed',
        msg: 'Fill in the required fields and fix the JSON schema.',
      });
      return;
    }

    setIsSaving(true);
    try {
      const workflow = getCurrentWorkflow();

      let saved_workflow_id = await saveWorkflow(workflow, workflowId);

      // Close modal first
      onClose(true);

      // Open workflow in a new window instead of navigating in the same window
      window.electron.createChatWindow(
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        saved_workflow_id
      );

      toastSuccess({
        title: workflow.title,
        msg: 'Workflow saved and started',
      });
    } catch (error) {
      console.error('Failed to save and run workflow:', error);

      toastError({
        title: 'Save and run failed',
        msg: `Failed to save and run workflow: ${error instanceof Error ? error.message : 'Unknown error'}`,
        traceback: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setIsSaving(false);
    }
  };

  if (!isOpen) return null;

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && !isSaving && onClose(false)}>
      <DialogContent
        dismissible={!isSaving}
        className="flex h-[90vh] w-[90vw] max-w-4xl flex-col gap-0 overflow-hidden p-0 sm:max-w-[90vw] lg:max-w-4xl"
      >
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-5 pr-14 border-b border-border-subtle flex-shrink-0">
          <div>
            <DialogTitle>{isCreateMode ? 'Create Workflow' : 'Edit Workflow'}</DialogTitle>
            <DialogDescription className="text-supporting text-text-muted mt-0.5">
              {isCreateMode
                ? 'Define agent behavior and capabilities for reusable chats.'
                : 'Edit the workflow to change agent behavior.'}{' '}
              <a
                href="http://biorouter.ucsf.edu/docs"
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-0.5 text-block-teal hover:underline"
              >
                Learn more
                <ExternalLink className="w-3 h-3" />
              </a>
            </DialogDescription>
          </div>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto px-6 py-4">
          <WorkflowFormFields
            form={form}
            extensions={workflowExtensions}
            onExtensionsChange={setWorkflowExtensions}
          />

          {/* Deep Link Display */}
          {requiredFieldsAreFilled() && (
            <div className="biorouter-modal-panel w-full p-4 rounded-container mt-6">
              <label
                htmlFor="workflow-share-link"
                className="block text-label text-text-default mb-2"
              >
                Share link
              </label>
              <div className="flex items-center">
                <Input
                  id="workflow-share-link"
                  type="text"
                  readOnly
                  value={
                    isGeneratingDeeplink
                      ? 'Generating link…'
                      : deeplinkError
                        ? 'Could not generate a share link'
                        : deeplink
                  }
                  onFocus={(e) => e.currentTarget.select()}
                  aria-invalid={deeplinkError}
                  className="flex-1 text-code font-mono"
                />
                {/* V7 — `ml-2` is the only class left, and it is spacing
                    BETWEEN this button and the field beside it rather than
                    geometry inside it. `px-3 py-2 rounded-element flex
                    items-center` was the primitive's own job four times over,
                    and the bare `flex` additionally flips the cva base's
                    `inline-flex` through tailwind-merge. */}
                <Button
                  type="button"
                  onClick={handleCopy}
                  variant="outline"
                  disabled={!canCopyDeeplink}
                  aria-label={copied ? 'Link copied' : 'Copy share link'}
                  className="ml-2"
                >
                  {copied ? (
                    <Check className="w-4 h-4 text-text-success" />
                  ) : (
                    <Copy className="w-4 h-4" />
                  )}
                </Button>
              </div>
              <p className="text-supporting text-text-muted mt-1">
                Anyone with this link can open this workflow in Biorouter. Paste it into a browser,
                or send it to a collaborator.
              </p>
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between p-6 border-t border-border-subtle">
          <Button
            onClick={() => onClose(false)}
            variant="ghost"
            disabled={isSaving}
            className="px-4 py-2 text-text-muted rounded-element hover:bg-overlay-hover transition-colors"
          >
            Close
          </Button>

          <div className="flex gap-3">
            <Button
              onClick={handleSaveWorkflowClick}
              disabled={!requiredFieldsAreFilled() || isSaving}
              variant="outline"
            >
              <Save className="w-4 h-4" />
              {isSaving ? 'Saving...' : 'Save Workflow'}
            </Button>
            <Button
              onClick={handleSaveAndRunWorkflowClick}
              disabled={!requiredFieldsAreFilled() || isSaving}
              variant="default"
            >
              <Play className="w-4 h-4" />
              {isSaving ? 'Saving...' : 'Save & Run Workflow'}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
