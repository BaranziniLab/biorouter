import React, { useState, useEffect } from 'react';
import { Parameter } from '../../../workflow';

import ParameterInput from '../../parameter/ParameterInput';
import WorkflowActivityEditor from '../WorkflowActivityEditor';
import JsonSchemaEditor from './JsonSchemaEditor';
import InstructionsEditor from './InstructionsEditor';
import { Button } from '../../ui/button';
import { Disclosure } from '../../ui/disclosure';
import { Field } from '../../ui/field';
import { InfoTip, useInfoTipId } from '../../ui/info-tip';
import { Input } from '../../ui/input';
import { Textarea } from '../../ui/textarea';
import { WorkflowFormApi, WorkflowFormData } from './workflowFormSchema';
import { getExtensions } from '../../../api';
import type { ExtensionConfig } from '../../../api';
import { WorkflowResourceItem, WorkflowResourcePicker } from './WorkflowResourcePicker';
import { storeSubscriptionCleanup } from '../../../utils/storeSubscription';
import { workflowFormCopy as copy } from '../copy';

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
  /** Shown beside the knowledge-base picker, e.g. why nothing in it is selected. */
  knowledgeBaseNotice?: string;
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

/**
 * A field whose label row also holds an action ("Open editor"): the `Field` recipe (a
 * `text-label` label, its InfoTip as the label's sibling, a 6px gap, the control, then one
 * `text-supporting` line, the error in danger ink), written out because `Field` has no slot
 * beside its label. The control is told about the help and the error by `aria-describedby`.
 */
function FieldWithAction({
  id,
  label,
  info,
  action,
  error,
  children,
}: {
  id: string;
  label: string;
  info?: string;
  action?: React.ReactNode;
  error?: React.ReactNode;
  children: React.ReactElement<Record<string, unknown>>;
}) {
  const infoId = useInfoTipId();
  const errorId = `${id}-help`;
  const control = React.cloneElement(children, {
    id,
    'aria-describedby':
      [info ? infoId : null, error ? errorId : null].filter(Boolean).join(' ') || undefined,
    'aria-invalid': error ? true : undefined,
  });
  return (
    <div className="br-form-field flex min-w-0 flex-col gap-1.5" data-slot="field">
      <div className="flex min-w-0 items-center justify-between gap-3">
        <div className="flex min-w-0 items-center">
          <label htmlFor={id} className="text-label text-text-default">
            {label}
          </label>
          {info && <InfoTip label={label} help={info} id={infoId} />}
        </div>
        {action}
      </div>
      {control}
      {error ? (
        <p id={errorId} className="text-supporting text-text-danger" data-slot="field-error">
          {error}
        </p>
      ) : null}
    </div>
  );
}

/** A group's label with its InfoTip: "Parameters", "Model". Sentence case, `text-label`. */
function GroupLabel({ id, label, info }: { id: string; label: string; info?: string }) {
  return (
    <div className="flex min-w-0 items-center">
      <span id={id} className="text-label text-text-default">
        {label}
      </span>
      {info && <InfoTip label={label} help={info} />}
    </div>
  );
}

/** "Initial prompt (optional)": optional fields are marked, required ones are not. */
function OptionalLabel({ children }: { children: string }) {
  return (
    <>
      {children} <span className="font-normal text-text-muted">{copy.optional}</span>
    </>
  );
}

export function WorkflowFormFields({
  form,
  extensions = [],
  onExtensionsChange,
  knowledgeBaseItems = [],
  selectedKnowledgeBaseIds = [],
  onKnowledgeBaseIdsChange,
  defaultKnowledgeBaseId,
  onDefaultKnowledgeBaseIdChange,
  knowledgeBaseNotice,
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

  const firstError = (field: FormFieldApi) =>
    field.state.meta.errors.length > 0 ? field.state.meta.errors[0] : undefined;
  const values = form.state.values;
  const advancedSummary = copy.advancedSummary(
    values.settings?.biorouter_model || null,
    values.parameters.filter((parameter: Parameter) => parameter.key?.trim()).length,
    extensions.length
  );

  return (
    <div className="flex flex-col gap-4" data-testid="workflow-form">
      <form.Field name="title">
        {(field: FormFieldApi<string>) => (
          <Field id="workflow-title" label={copy.title} error={firstError(field)} required>
            <Input
              type="text"
              value={field.state.value}
              onChange={(e) => {
                field.handleChange(e.target.value);
                onTitleChange?.(e.target.value);
              }}
              onBlur={field.handleBlur}
              placeholder={copy.titlePlaceholder}
              data-testid="title-input"
            />
          </Field>
        )}
      </form.Field>

      <form.Field name="description">
        {(field: FormFieldApi<string>) => (
          <Field
            id="workflow-description"
            label={copy.description}
            error={firstError(field)}
            required
          >
            <Input
              type="text"
              value={field.state.value}
              onChange={(e) => {
                field.handleChange(e.target.value);
                onDescriptionChange?.(e.target.value);
              }}
              onBlur={field.handleBlur}
              placeholder={copy.descriptionPlaceholder}
              data-testid="description-input"
            />
          </Field>
        )}
      </form.Field>

      <form.Field name="instructions">
        {(field: FormFieldApi<string>) => (
          <>
            <FieldWithAction
              id="workflow-instructions"
              label={copy.instructions}
              info={copy.instructionsInfo}
              error={firstError(field)}
              action={
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={() => setShowInstructionsEditor(true)}
                >
                  {copy.openEditor}
                </Button>
              }
            >
              <Textarea
                value={field.state.value}
                onChange={(e) => {
                  field.handleChange(e.target.value);
                  onInstructionsChange?.(e.target.value);
                }}
                onBlur={() => {
                  field.handleBlur();
                  updateParametersFromFields();
                }}
                placeholder={copy.instructionsPlaceholder}
                rows={8}
                data-testid="instructions-input"
              />
            </FieldWithAction>

            <InstructionsEditor
              isOpen={showInstructionsEditor}
              onClose={() => setShowInstructionsEditor(false)}
              value={field.state.value}
              onChange={(value) => {
                field.handleChange(value);
                onInstructionsChange?.(value);
                updateParametersFromFields();
              }}
              error={firstError(field)}
            />
          </>
        )}
      </form.Field>

      <form.Field name="prompt">
        {(field: FormFieldApi<string | undefined>) => (
          <Field
            id="workflow-prompt"
            label={<OptionalLabel>{copy.prompt}</OptionalLabel>}
            info={copy.promptInfo}
          >
            <Textarea
              value={field.state.value || ''}
              onChange={(e) => {
                field.handleChange(e.target.value);
                onPromptChange?.(e.target.value);
              }}
              onBlur={() => {
                field.handleBlur();
                updateParametersFromFields();
              }}
              placeholder={copy.promptPlaceholder}
              rows={3}
              data-testid="prompt-input"
            />
          </Field>
        )}
      </form.Field>

      {/* The closed section states its defaults, so nothing it hides is a surprise. */}
      <Disclosure
        label={copy.advanced}
        summary={advancedSummary}
        open={advancedOpen}
        onOpenChange={setAdvancedOpen}
      >
        <div className="flex flex-col gap-5 pt-2">
          <form.Field name="activities">
            {(field: FormFieldApi<string[]>) => (
              <WorkflowActivityEditor
                activities={field.state.value}
                setActivities={(activities) => field.handleChange(activities)}
                onBlur={updateParametersFromFields}
              />
            )}
          </form.Field>

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

              const handleDeleteParameter = (parameterKey: string) => {
                const updatedParams = field.state.value.filter(
                  (param: Parameter) => param.key !== parameterKey
                );
                field.handleChange(updatedParams);
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
                <div
                  className="flex flex-col gap-1.5"
                  role="group"
                  aria-labelledby="workflow-parameters-label"
                >
                  <GroupLabel
                    id="workflow-parameters-label"
                    label={copy.parameters}
                    info={copy.parametersInfo}
                  />
                  <div className="flex gap-2">
                    <Input
                      type="text"
                      value={newParameterName}
                      onChange={(e) => setNewParameterName(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') {
                          e.preventDefault();
                          handleAddParameter();
                        }
                      }}
                      aria-label={copy.parameterPlaceholder}
                      placeholder={copy.parameterPlaceholder}
                    />
                    <Button
                      type="button"
                      variant="secondary"
                      onClick={handleAddParameter}
                      disabled={!newParameterName.trim()}
                    >
                      {copy.addParameter}
                    </Button>
                  </div>

                  {field.state.value
                    .filter((parameter: Parameter) => parameter.key && parameter.key.trim())
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

          <form.Field name="jsonSchema">
            {(field: FormFieldApi<string | undefined>) => (
              <div
                className="flex flex-col gap-1.5"
                role="group"
                aria-labelledby="workflow-schema-label"
              >
                <div className="flex min-w-0 items-center justify-between gap-3">
                  <GroupLabel
                    id="workflow-schema-label"
                    label={copy.responseSchema}
                    info={copy.responseSchemaInfo}
                  />
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => setShowJsonSchemaEditor(true)}
                  >
                    {copy.openEditor}
                  </Button>
                </div>

                {field.state.value && field.state.value.trim() && (
                  <pre
                    className={`max-h-32 overflow-y-auto whitespace-pre-wrap break-words rounded-element border bg-background-muted p-3 font-mono text-code text-text-default ${
                      field.state.meta.errors.length > 0
                        ? 'border-border-danger'
                        : 'border-border-subtle'
                    }`}
                  >
                    {field.state.value}
                  </pre>
                )}

                {firstError(field) && (
                  <p className="text-supporting text-text-danger">{firstError(field)}</p>
                )}

                <JsonSchemaEditor
                  isOpen={showJsonSchemaEditor}
                  onClose={() => setShowJsonSchemaEditor(false)}
                  value={field.state.value || ''}
                  onChange={(value) => {
                    field.handleChange(value);
                    onJsonSchemaChange?.(value);
                  }}
                  error={firstError(field)}
                />
              </div>
            )}
          </form.Field>

          {/* The model override: three fields, no tinted block. Blank means the
              person's default model, which the group's help says once. */}
          <div
            className="flex flex-col gap-1.5"
            role="group"
            aria-labelledby="workflow-model-label"
          >
            <GroupLabel id="workflow-model-label" label={copy.model} info={copy.modelInfo} />
            <div className="grid grid-cols-3 gap-3">
              <form.Field name="settings.biorouter_provider">
                {(field: FormFieldApi<string | undefined>) => (
                  <Field id="workflow-provider" label={copy.provider}>
                    <Input
                      type="text"
                      value={field.state.value || ''}
                      onChange={(e) => field.handleChange(e.target.value || undefined)}
                      placeholder={copy.defaultPlaceholder}
                    />
                  </Field>
                )}
              </form.Field>
              <form.Field name="settings.biorouter_model">
                {(field: FormFieldApi<string | undefined>) => (
                  <Field id="workflow-model" label={copy.modelName}>
                    <Input
                      type="text"
                      value={field.state.value || ''}
                      onChange={(e) => field.handleChange(e.target.value || undefined)}
                      placeholder={copy.defaultPlaceholder}
                    />
                  </Field>
                )}
              </form.Field>
              <form.Field name="settings.temperature">
                {(field: FormFieldApi<number | undefined>) => (
                  <Field id="workflow-temperature" label={copy.temperature}>
                    <Input
                      type="number"
                      min={0}
                      max={2}
                      step={0.1}
                      value={field.state.value ?? ''}
                      onChange={(e) => {
                        const v = e.target.value;
                        field.handleChange(v === '' ? undefined : parseFloat(v));
                      }}
                      placeholder={copy.temperaturePlaceholder}
                      className="tabular-nums"
                    />
                  </Field>
                )}
              </form.Field>
            </div>
          </div>

          {onKnowledgeBaseIdsChange && (
            <WorkflowResourcePicker
              label={copy.knowledgeBases}
              description={copy.knowledgeBasesInfo}
              items={knowledgeBaseItems}
              selectedIds={selectedKnowledgeBaseIds}
              onSelectedIdsChange={onKnowledgeBaseIdsChange}
              defaultId={defaultKnowledgeBaseId}
              onDefaultIdChange={onDefaultKnowledgeBaseIdChange}
              // A chat with no pinned primary is captured as `default: null`,
              // and the daemon never infers one from `visible`
              // (`plan_knowledge_selection`), so the workflow really will have
              // no default, and every chat it starts will have no write target.
              // That is the intent, not a bug, so the card says "No default" in
              // words and what it costs in that line's help.
              noDefaultText={copy.noDefaultKnowledgeBase}
              notice={knowledgeBaseNotice}
              emptyText={copy.noKnowledgeBases}
              searchPlaceholder={copy.searchKnowledgeBases}
              noun="KB"
            />
          )}

          {onSkillIdsChange && (
            <WorkflowResourcePicker
              label={copy.skills}
              description={copy.skillsInfo}
              items={skillItems}
              selectedIds={selectedSkillIds}
              onSelectedIdsChange={onSkillIdsChange}
              emptyText={copy.noSkills}
              searchPlaceholder={copy.searchSkills}
              noun="skill"
            />
          )}

          {onExtensionsChange && (
            <WorkflowResourcePicker
              label={copy.extensions}
              description={copy.extensionsInfo}
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
              emptyText={copy.noExtensions}
              searchPlaceholder={copy.searchExtensions}
              noun="extension"
            />
          )}
        </div>
      </Disclosure>
    </div>
  );
}
