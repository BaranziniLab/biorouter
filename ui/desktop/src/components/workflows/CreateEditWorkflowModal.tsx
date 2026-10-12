import React, { useState, useEffect, useCallback } from 'react';
import { useForm } from '@tanstack/react-form';
import { Workflow, Parameter } from '../../workflow';
import { ExtensionConfig } from '../ConfigContext';
import { Button } from '../ui/button';
import { ModalShell } from '../ModalShell';

import { WorkflowFormFields } from './shared/WorkflowFormFields';
import { WorkflowFormData } from './shared/workflowFormSchema';
import { toastSuccess, toastError } from '../../toasts';
import { saveWorkflow } from '../../workflow/workflow_management';
import { storeSubscriptionCleanup } from '../../utils/storeSubscription';
import { workflowFormCopy as copy } from './copy';

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

  /** Why `validateForm` said no, in one line: the missing fields, or the schema. */
  const validationMessage = () => (requiredFieldsAreFilled() ? copy.invalidSchema : copy.invalid);

  const savedTitle = (title?: string) =>
    isCreateMode ? copy.created((title || '').trim()) : copy.saved((title || '').trim());

  const handleSaveWorkflowClick = async () => {
    if (isSaving) return;
    if (!validateForm()) {
      toastError({ title: copy.couldNotSave, msg: validationMessage() });
      return;
    }

    setIsSaving(true);
    try {
      const workflow = getCurrentWorkflow();

      await saveWorkflow(workflow, workflowId);

      onClose(true);

      toastSuccess({ title: savedTitle(workflow.title) });
    } catch (error) {
      console.error('Failed to save workflow:', error);

      toastError({
        title: copy.couldNotSave,
        msg: error instanceof Error ? error.message : String(error),
        traceback: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setIsSaving(false);
    }
  };

  const handleSaveAndRunWorkflowClick = async () => {
    if (isSaving) return;
    if (!validateForm()) {
      toastError({ title: copy.couldNotSave, msg: validationMessage() });
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

      toastSuccess({ title: savedTitle(workflow.title) });
    } catch (error) {
      console.error('Failed to save and run workflow:', error);

      toastError({
        title: copy.couldNotSave,
        msg: error instanceof Error ? error.message : String(error),
        traceback: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setIsSaving(false);
    }
  };

  if (!isOpen) return null;

  // A workflow that does not exist yet is created, not saved (spec 3.10); the same two buttons
  // read Save and Save and run once there is a file to write over.
  const progressLabel = isCreateMode ? copy.creating : copy.saving;

  // One editor shape (spec 3.10): `ModalShell size="lg" scrollBody`, a title and nothing under
  // it. Sharing is not part of editing; it lives on the Workflows row menu ("Copy link").
  return (
    <ModalShell
      open={isOpen}
      onOpenChange={(open) => {
        if (!open && !isSaving) onClose(false);
      }}
      size="lg"
      purpose="form"
      scrollBody
      title={isCreateMode ? copy.newTitle : copy.editTitle}
      footer={
        <>
          <Button onClick={() => onClose(false)} variant="secondary" disabled={isSaving}>
            {copy.cancel}
          </Button>
          <Button
            onClick={handleSaveWorkflowClick}
            disabled={!requiredFieldsAreFilled() || isSaving}
            variant="secondary"
          >
            {isSaving ? progressLabel : isCreateMode ? copy.create : copy.save}
          </Button>
          <Button
            onClick={handleSaveAndRunWorkflowClick}
            disabled={!requiredFieldsAreFilled() || isSaving}
          >
            {isSaving ? progressLabel : isCreateMode ? copy.createAndRun : copy.saveAndRun}
          </Button>
        </>
      }
    >
      <div className="py-4">
        <WorkflowFormFields
          form={form}
          extensions={workflowExtensions}
          onExtensionsChange={setWorkflowExtensions}
        />
      </div>
    </ModalShell>
  );
}
