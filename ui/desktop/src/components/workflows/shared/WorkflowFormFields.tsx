import React, { useState, useEffect } from 'react';
import { Parameter } from '../../../workflow';
import { ChevronDown } from '../../icons/app-icons';

import ParameterInput from '../../parameter/ParameterInput';
import WorkflowActivityEditor from '../WorkflowActivityEditor';
import JsonSchemaEditor from './JsonSchemaEditor';
import InstructionsEditor from './InstructionsEditor';
import { Button } from '../../ui/button';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '../../ui/collapsible';
import { WorkflowFormApi, WorkflowFormData } from './workflowFormSchema';
import { getExtensions } from '../../../api';
import type { ExtensionConfig } from '../../../api';
import { WorkflowResourceItem, WorkflowResourcePicker } from './WorkflowResourcePicker';
import { storeSubscriptionCleanup } from '../../../utils/storeSubscription';

// Type for field API to avoid linting issues - use any to bypass complex type constraints
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type FormFieldApi<_T = any> = any;

interface WorkflowFormFieldsProps {
  // Form instance from parent
  form: WorkflowFormApi;

  // Extensions state managed by parent
  extensions?: ExtensionConfig[];
  onExtensionsChange?: (exts: ExtensionConfig[]) => void;
  knowledgeBaseItems?: WorkflowResourceItem[];
  selectedKnowledgeBaseIds?: string[];
  onKnowledgeBaseIdsChange?: (ids: string[]) => void;
  defaultKnowledgeBaseId?: string | null;
  onDefaultKnowledgeBaseIdChange?: (id: string | null) => void;
  skillItems?: WorkflowResourceItem[];
  selectedSkillIds?: string[];
  onSkillIdsChange?: (ids: string[]) => void;

  // Event handlers
  onTitleChange?: (value: string) => void;
  onDescriptionChange?: (value: string) => void;
  onInstructionsChange?: (value: string) => void;
  onPromptChange?: (value: string) => void;
  onJsonSchemaChange?: (value: string) => void;
}

export const extractTemplateVariables = (content: string): string[] => {
  const templateVarRegex = /\{\{(.*?)\}\}/g;
  const variables: string[] = [];
  let match;

  while ((match = templateVarRegex.exec(content)) !== null) {
    const variable = match[1].trim();

    if (variable && !variables.includes(variable)) {
      // Filter out complex variables that aren't valid parameter names
      // This matches the backend logic in filter_complex_variables()
      const validVarRegex = /^\s*[a-zA-Z_][a-zA-Z0-9_]*\s*$/;
      if (validVarRegex.test(variable)) {
        variables.push(variable);
      }
    }
  }

  return variables;
};

export function WorkflowFormFields({
  form,
  extensions = [],
  onExtensionsChange,
  knowledgeBaseItems = [],
  selectedKnowledgeBaseIds = [],
  onKnowledgeBaseIdsChange,
  defaultKnowledgeBaseId,
  onDefaultKnowledgeBaseIdChange,
  skillItems = [],
  selectedSkillIds = [],
  onSkillIdsChange,
  onTitleChange,
  onDescriptionChange,
  onInstructionsChange,
  onPromptChange,
  onJsonSchemaChange,
}: WorkflowFormFieldsProps) {
  const [showJsonSchemaEditor, setShowJsonSchemaEditor] = useState(false);
  const [showInstructionsEditor, setShowInstructionsEditor] = useState(false);
  const [newParameterName, setNewParameterName] = useState('');
  const [expandedParameters, setExpandedParameters] = useState<Set<string>>(new Set());
  const [availableExtensions, setAvailableExtensions] = useState<ExtensionConfig[]>([]);
  const inputClass =
    'w-full h-9 rounded-element border-0 bg-background-default px-3 text-body text-text-default placeholder:text-text-muted ring-1 ring-border-input transition-colors hover:bg-overlay-hover focus:bg-background-default ';
  const textareaClass =
    'w-full rounded-element border-0 bg-background-default px-3 py-2 text-body text-text-default placeholder:text-text-muted ring-1 ring-border-input transition-colors hover:bg-overlay-hover focus:bg-background-default resize-none';

  useEffect(() => {
    getExtensions({ throwOnError: false }).then((res) => {
      if (res.data?.extensions) {
        setAvailableExtensions(res.data.extensions.map((e) => ({ ...e })));
      }
    });
  }, []);

  // Force re-render when instructions, prompt, or activities change
  const [_forceRender, setForceRender] = useState(0);

  React.useEffect(() => {
    const subscription = form.store.subscribe(() => {
      // Force re-render when any form field changes to update parameter usage indicators
      setForceRender((prev) => prev + 1);
    });
    return storeSubscriptionCleanup(subscription);
  }, [form.store]);

  const parseParametersFromInstructions = React.useCallback(
    (instructions: string, prompt?: string, activities?: string[]): Parameter[] => {
      const instructionVars = extractTemplateVariables(instructions);
      const promptVars = prompt ? extractTemplateVariables(prompt) : [];
      const activityVars = activities
        ? activities.flatMap((activity) => extractTemplateVariables(activity))
        : [];

      // Combine and deduplicate
      const allVars = [...new Set([...instructionVars, ...promptVars, ...activityVars])];

      return allVars.map((key: string) => ({
        key,
        description: `Enter value for ${key}`,
        requirement: 'required' as const,
        input_type: 'string' as const,
      }));
    },
    []
  );

  // Function to update parameters based on current field values
  const updateParametersFromFields = React.useCallback(() => {
    const currentValues = form.state.values;
    const { instructions, prompt, activities, parameters: currentParams } = currentValues;

    const newParams = parseParametersFromInstructions(instructions, prompt, activities);

    // Separate manually added parameters (those not found in instructions/prompt/activities)
    const manualParams = currentParams.filter((param: Parameter) => {
      // Only keep manual params that have a valid key and are not found in the parsed params
      return (
        param.key && param.key.trim() && !newParams.some((newParam) => newParam.key === param.key)
      );
    });

    // Combine parsed parameters with manually added ones, filtering out empty ones
    const combinedParams = [
      ...newParams.map((newParam) => {
        const existing = currentParams.find((cp: Parameter) => cp.key === newParam.key);
        return existing ? { ...existing } : newParam;
      }),
      ...manualParams,
    ].filter((param: Parameter) => param.key && param.key.trim()) as Parameter[];

    // Only update if parameters actually changed
    const currentParamKeys = currentParams.map((p: Parameter) => p.key).sort();
    const newParamKeys = combinedParams.map((p) => p.key).sort();

    if (JSON.stringify(currentParamKeys) !== JSON.stringify(newParamKeys)) {
      form.setFieldValue('parameters', combinedParams);
    }
  }, [form, parseParametersFromInstructions]);

  const isParameterUsed = (
    paramKey: string,
    instructions: string,
    prompt?: string,
    activities?: string[]
  ): boolean => {
    const regex = new RegExp(
      `\\{\\{\\s*${paramKey.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\}\\}`,
      'g'
    );
    const usedInInstructions = regex.test(instructions);
    const usedInPrompt = prompt ? regex.test(prompt) : false;
    const usedInActivities = activities
      ? activities.some((activity) => {
          // For activities, we need to check the full activity string, including message: prefixes
          return regex.test(activity);
        })
      : false;
    return usedInInstructions || usedInPrompt || usedInActivities;
  };

  const checkHasAdvancedData = React.useCallback(
    (values: WorkflowFormData) => {
      const hasActivities = Boolean(values.activities && values.activities.length > 0);
      const hasParameters = Boolean(values.parameters && values.parameters.length > 0);
      const hasJsonSchema = Boolean(values.jsonSchema && values.jsonSchema.trim());
      const hasSettings = Boolean(
        values.settings?.biorouter_provider ||
        values.settings?.biorouter_model ||
        values.settings?.temperature !== undefined
      );
      const hasExtensions = extensions.length > 0;
      const hasKnowledgeBases = selectedKnowledgeBaseIds.length > 0;
      const hasSkills = selectedSkillIds.length > 0;
      return (
        hasActivities ||
        hasParameters ||
        hasJsonSchema ||
        hasSettings ||
        hasExtensions ||
        hasKnowledgeBases ||
        hasSkills
      );
    },
    [extensions, selectedKnowledgeBaseIds.length, selectedSkillIds.length]
  );

  const [advancedOpen, setAdvancedOpen] = useState(() => checkHasAdvancedData(form.state.values));
  const hasAutoOpenedForExtensions = React.useRef(false);

  // Auto-open Advanced Options when pre-loaded extensions arrive (e.g. from session)
  useEffect(() => {
    if (
      (extensions.length > 0 ||
        selectedKnowledgeBaseIds.length > 0 ||
        selectedSkillIds.length > 0) &&
      !hasAutoOpenedForExtensions.current
    ) {
      hasAutoOpenedForExtensions.current = true;
      setAdvancedOpen(true);
    }
  }, [extensions, selectedKnowledgeBaseIds.length, selectedSkillIds.length]);

  useEffect(() => {
    const parameterKeys = form.state.values.parameters
      .map((parameter: Parameter) => parameter.key)
      .filter((key: string | undefined): key is string => Boolean(key && key.trim()));

    setExpandedParameters((prev) => {
      const next = new Set(prev);
      let changed = false;

      for (const key of parameterKeys) {
        if (!next.has(key)) {
          next.add(key);
          changed = true;
        }
      }

      for (const key of [...next]) {
        if (!parameterKeys.includes(key)) {
          next.delete(key);
          changed = true;
        }
      }

      return changed ? next : prev;
    });
  }, [form.state.values.parameters]);

  return (
    <div className="space-y-4" data-testid="workflow-form">
      {/* Title Field */}
      <form.Field name="title">
        {(field: FormFieldApi<string>) => (
          <div>
            <label htmlFor="workflow-title" className="block text-label text-text-default mb-2">
              Title <span className="text-text-danger">*</span>
            </label>
            <input
              id="workflow-title"
              type="text"
              value={field.state.value}
              onChange={(e) => {
                field.handleChange(e.target.value);
                onTitleChange?.(e.target.value);
              }}
              onBlur={field.handleBlur}
              className={`${inputClass} ${field.state.meta.errors.length > 0 ? 'ring-2 ring-border-danger' : ''}`}
              placeholder="Workflow title"
              data-testid="title-input"
            />
            {field.state.meta.errors.length > 0 && (
              <p className="text-text-danger text-body mt-1">{field.state.meta.errors[0]}</p>
            )}
          </div>
        )}
      </form.Field>

      {/* Description Field */}
      <form.Field name="description">
        {(field: FormFieldApi<string>) => (
          <div>
            <label
              htmlFor="workflow-description"
              className="block text-label text-text-default mb-2"
            >
              Description <span className="text-text-danger">*</span>
            </label>
            <input
              id="workflow-description"
              type="text"
              value={field.state.value}
              onChange={(e) => {
                field.handleChange(e.target.value);
                onDescriptionChange?.(e.target.value);
              }}
              onBlur={field.handleBlur}
              className={`${inputClass} ${field.state.meta.errors.length > 0 ? 'ring-2 ring-border-danger' : ''}`}
              placeholder="Brief description of what this workflow does"
              data-testid="description-input"
            />
            {field.state.meta.errors.length > 0 && (
              <p className="text-text-danger text-body mt-1">{field.state.meta.errors[0]}</p>
            )}
          </div>
        )}
      </form.Field>

      {/* Instructions Field */}
      <form.Field name="instructions">
        {(field: FormFieldApi<string>) => (
          <div>
            <div className="flex items-center justify-between mb-2">
              <label htmlFor="workflow-instructions" className="block text-label text-text-default">
                Instructions <span className="text-text-danger">*</span>
              </label>
              <Button
                type="button"
                onClick={() => setShowInstructionsEditor(true)}
                variant="ghost"
                size="sm"
                className="h-7 rounded-element bg-background-medium px-2 text-supporting hover:bg-overlay-hover"
              >
                Open editor
              </Button>
            </div>
            <textarea
              id="workflow-instructions"
              value={field.state.value}
              onChange={(e) => {
                field.handleChange(e.target.value);
                onInstructionsChange?.(e.target.value);
              }}
              onBlur={() => {
                field.handleBlur();
                updateParametersFromFields();
              }}
              className={`${textareaClass} ${field.state.meta.errors.length > 0 ? 'ring-2 ring-border-danger' : ''}`}
              placeholder="Detailed instructions for the AI, hidden from the user"
              rows={8}
              data-testid="instructions-input"
            />
            <p className="text-supporting text-text-muted mt-1">
              Use {`{{parameter_name}}`} to define parameters that can be filled in when running the
              workflow.
            </p>
            {field.state.meta.errors.length > 0 && (
              <p className="text-text-danger text-body mt-1">{field.state.meta.errors[0]}</p>
            )}

            {/* Instructions Editor Modal */}
            <InstructionsEditor
              isOpen={showInstructionsEditor}
              onClose={() => setShowInstructionsEditor(false)}
              value={field.state.value}
              onChange={(value) => {
                field.handleChange(value);
                onInstructionsChange?.(value);
                updateParametersFromFields();
              }}
              error={field.state.meta.errors.length > 0 ? field.state.meta.errors[0] : undefined}
            />
          </div>
        )}
      </form.Field>

      {/* Initial prompt field */}
      <form.Field name="prompt">
        {(field: FormFieldApi<string | undefined>) => (
          <div>
            <label htmlFor="workflow-prompt" className="block text-label text-text-default mb-2">
              Initial prompt
            </label>
            <p className="text-supporting text-text-muted mt-2 mb-2">
              (optional — instructions or a prompt are required)
            </p>
            <textarea
              id="workflow-prompt"
              value={field.state.value || ''}
              onChange={(e) => {
                field.handleChange(e.target.value);
                onPromptChange?.(e.target.value);
              }}
              onBlur={() => {
                field.handleBlur();
                updateParametersFromFields();
              }}
              className={textareaClass}
              placeholder="Pre-filled prompt when the workflow starts"
              rows={3}
              data-testid="prompt-input"
            />
          </div>
        )}
      </form.Field>

      {/* Advanced Section - Collapsible */}
      <Collapsible open={advancedOpen} onOpenChange={setAdvancedOpen} className="mt-6">
        <CollapsibleTrigger className="flex w-full items-baseline gap-2 rounded-element bg-background-medium px-3 py-2.5 transition-colors hover:bg-overlay-hover">
          <ChevronDown
            className={`w-4 h-4 text-text-muted transition-transform flex-shrink-0 relative top-0.5 ${advancedOpen ? 'rotate-0' : '-rotate-90'}`}
          />
          <span className="text-label text-text-default">Advanced options</span>
          <span className="text-supporting text-text-muted">
            Activities, parameters, model, resources
          </span>
        </CollapsibleTrigger>

        <CollapsibleContent className="mt-4 space-y-5">
          {/* Activities Field */}
          <form.Field name="activities">
            {(field: FormFieldApi<string[]>) => (
              <div>
                <WorkflowActivityEditor
                  activities={field.state.value}
                  setActivities={(activities) => field.handleChange(activities)}
                  onBlur={updateParametersFromFields}
                />
              </div>
            )}
          </form.Field>

          {/* Parameters Field */}
          <form.Field name="parameters">
            {(field: FormFieldApi<Parameter[]>) => {
              const handleAddParameter = () => {
                if (newParameterName.trim()) {
                  const newParam: Parameter = {
                    key: newParameterName.trim(),
                    description: `Enter value for ${newParameterName.trim()}`,
                    input_type: 'string',
                    requirement: 'required',
                  };
                  field.handleChange([...field.state.value, newParam]);
                  setNewParameterName('');
                  // Expand the newly added parameter by default
                  setExpandedParameters((prev) => {
                    const newSet = new Set(prev);
                    newSet.add(newParam.key);
                    return newSet;
                  });
                }
              };

              const handleKeyPress = (e: React.KeyboardEvent) => {
                if (e.key === 'Enter') {
                  e.preventDefault();
                  handleAddParameter();
                }
              };

              const handleDeleteParameter = (parameterKey: string) => {
                const updatedParams = field.state.value.filter(
                  (param: Parameter) => param.key !== parameterKey
                );
                field.handleChange(updatedParams);
                // Remove from expanded set if it was expanded
                setExpandedParameters((prev) => {
                  const newSet = new Set(prev);
                  newSet.delete(parameterKey);
                  return newSet;
                });
              };

              const handleToggleExpanded = (parameterKey: string) => {
                setExpandedParameters((prev) => {
                  const newSet = new Set(prev);
                  if (newSet.has(parameterKey)) {
                    newSet.delete(parameterKey);
                  } else {
                    newSet.add(parameterKey);
                  }
                  return newSet;
                });
              };

              return (
                <div>
                  <label className="block text-label text-text-default mb-1">Parameters</label>
                  <p className="text-text-muted text-body space-y-2 pb-4">
                    Parameters will be automatically detected from {`{{parameter_name}}`} syntax in
                    instructions/prompt/activities or you can manually add them below.
                  </p>

                  {/* Add Parameter Input - Always Visible */}
                  <div className="flex gap-2 mb-4">
                    <input
                      type="text"
                      value={newParameterName}
                      onChange={(e) => setNewParameterName(e.target.value)}
                      onKeyPress={handleKeyPress}
                      placeholder="Enter parameter name..."
                      className={inputClass}
                    />
                    <Button
                      type="button"
                      onClick={handleAddParameter}
                      disabled={!newParameterName.trim()}
                      variant="ghost"
                      size="sm"
                      className="rounded-element bg-background-medium hover:bg-overlay-hover"
                    >
                      Add parameter
                    </Button>
                  </div>

                  {field.state.value.length > 0 &&
                    field.state.value
                      .filter((parameter: Parameter) => parameter.key && parameter.key.trim()) // Filter out empty parameters
                      .map((parameter: Parameter) => {
                        const currentValues = form.state.values;
                        const isUnused = !isParameterUsed(
                          parameter.key,
                          currentValues.instructions,
                          currentValues.prompt,
                          currentValues.activities
                        );

                        return (
                          <ParameterInput
                            key={parameter.key}
                            parameter={parameter}
                            isUnused={isUnused}
                            isExpanded={expandedParameters.has(parameter.key)}
                            onToggleExpanded={handleToggleExpanded}
                            onDelete={handleDeleteParameter}
                            onChange={(name, value) => {
                              const updatedParams = field.state.value.map((param: Parameter) =>
                                param.key === name ? { ...param, ...value } : param
                              );
                              field.handleChange(updatedParams);
                            }}
                          />
                        );
                      })}
                </div>
              );
            }}
          </form.Field>

          {/* JSON Schema Field */}
          <form.Field name="jsonSchema">
            {(field: FormFieldApi<string | undefined>) => (
              <div>
                <label className="block text-label text-text-default mb-1">
                  Response JSON Schema
                </label>
                <p className="text-text-muted text-body space-y-2 pb-4">
                  Define the expected structure of the AI's response using JSON Schema format
                </p>
                <div className="flex items-center justify-between mb-2">
                  <Button
                    type="button"
                    onClick={() => setShowJsonSchemaEditor(true)}
                    variant="ghost"
                    size="sm"
                    className="h-7 rounded-element bg-background-medium px-2 text-supporting hover:bg-overlay-hover"
                  >
                    Open editor
                  </Button>
                </div>

                {field.state.value && field.state.value.trim() && (
                  <div
                    className={`rounded-element bg-background-medium p-3 ${field.state.meta.errors.length > 0 ? 'ring-2 ring-border-danger' : ''}`}
                  >
                    <pre className="text-code font-mono text-text-default whitespace-pre-wrap break-words max-h-32 overflow-y-auto">
                      {field.state.value}
                    </pre>
                  </div>
                )}

                {field.state.meta.errors.length > 0 && (
                  <p className="text-text-danger text-body mt-1">{field.state.meta.errors[0]}</p>
                )}

                {/* JSON Schema Editor Modal */}
                <JsonSchemaEditor
                  isOpen={showJsonSchemaEditor}
                  onClose={() => setShowJsonSchemaEditor(false)}
                  value={field.state.value || ''}
                  onChange={(value) => {
                    field.handleChange(value);
                    onJsonSchemaChange?.(value);
                  }}
                  error={
                    field.state.meta.errors.length > 0 ? field.state.meta.errors[0] : undefined
                  }
                />
              </div>
            )}
          </form.Field>

          {/* Model Settings */}
          <div className="space-y-3 rounded-element bg-background-muted/60 p-3">
            <div className="flex items-baseline justify-between gap-3">
              <div>
                <label className="block text-label text-text-default">Model settings</label>
                <p className="mt-0.5 text-supporting text-text-muted">
                  Optional workflow-specific provider, model, and temperature.
                </p>
              </div>
              <span className="shrink-0 text-supporting text-text-muted">
                Global default if blank
              </span>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <form.Field name="settings.biorouter_provider">
                {(field: FormFieldApi<string | undefined>) => (
                  <div>
                    <label className="block text-caps text-text-muted mb-1">Provider</label>
                    <input
                      type="text"
                      value={field.state.value || ''}
                      onChange={(e) => field.handleChange(e.target.value || undefined)}
                      // Local providers first, matching the ordering every
                      // other provider surface uses (Local before Institutional
                      // before Commercial). The old placeholder named only
                      // "anthropic, openai" and so silently implied the
                      // subscription-backed and local providers were not
                      // options here.
                      placeholder="e.g. llamacpp, ollama, anthropic, claude_code"
                      className={inputClass}
                    />
                  </div>
                )}
              </form.Field>
              <form.Field name="settings.biorouter_model">
                {(field: FormFieldApi<string | undefined>) => (
                  <div>
                    <label className="block text-caps text-text-muted mb-1">Model</label>
                    <input
                      type="text"
                      value={field.state.value || ''}
                      onChange={(e) => field.handleChange(e.target.value || undefined)}
                      // A model id, not a model that must exist: the field is
                      // free text and the provider decides. The old example
                      // named a model from a superseded generation.
                      placeholder="e.g. claude-opus-5, gemma4-12b"
                      className={inputClass}
                    />
                  </div>
                )}
              </form.Field>
            </div>
            <form.Field name="settings.temperature">
              {(field: FormFieldApi<number | undefined>) => (
                <div>
                  <label className="block text-caps text-text-muted mb-1">
                    Temperature{' '}
                    <span className="normal-case font-normal">(0 to 2, blank = default)</span>
                  </label>
                  <input
                    type="number"
                    min={0}
                    max={2}
                    step={0.1}
                    value={field.state.value ?? ''}
                    onChange={(e) => {
                      const v = e.target.value;
                      field.handleChange(v === '' ? undefined : parseFloat(v));
                    }}
                    placeholder="e.g. 0.7"
                    className={inputClass}
                  />
                </div>
              )}
            </form.Field>
          </div>

          {(onKnowledgeBaseIdsChange || onSkillIdsChange || onExtensionsChange) && (
            <div className="space-y-4">
              {onKnowledgeBaseIdsChange && (
                <WorkflowResourcePicker
                  label="Knowledge bases"
                  description="Choose which knowledge bases this workflow can search and which one is focused by default."
                  items={knowledgeBaseItems}
                  selectedIds={selectedKnowledgeBaseIds}
                  onSelectedIdsChange={onKnowledgeBaseIdsChange}
                  defaultId={defaultKnowledgeBaseId}
                  onDefaultIdChange={onDefaultKnowledgeBaseIdChange}
                  emptyText="No knowledge bases found"
                  searchPlaceholder="Search knowledge bases..."
                  noun="KB"
                />
              )}

              {onSkillIdsChange && (
                <WorkflowResourcePicker
                  label="Skills"
                  description="Choose the skills this workflow should prefer when the skills extension is enabled."
                  items={skillItems}
                  selectedIds={selectedSkillIds}
                  onSelectedIdsChange={onSkillIdsChange}
                  emptyText="No skills found"
                  searchPlaceholder="Search skills..."
                  noun="skill"
                />
              )}

              {onExtensionsChange && (
                <WorkflowResourcePicker
                  label="Extensions"
                  description="Choose which extensions are active for this workflow. Empty means Biorouter uses the global default."
                  items={availableExtensions.map((ext) => ({
                    id: ext.name,
                    label: ext.name,
                    description: ext.description,
                  }))}
                  selectedIds={extensions.map((ext) => ext.name)}
                  onSelectedIdsChange={(ids) => {
                    const selected = availableExtensions.filter((ext) => ids.includes(ext.name));
                    onExtensionsChange(selected);
                  }}
                  emptyText="No extensions configured"
                  searchPlaceholder="Search extensions..."
                  noun="extension"
                />
              )}
            </div>
          )}
        </CollapsibleContent>
      </Collapsible>
    </div>
  );
}
