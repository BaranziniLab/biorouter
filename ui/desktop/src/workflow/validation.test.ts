import { describe, it, expect } from 'vitest';
import { getWorkflowJsonSchema } from './validation';

describe('Workflow Validation', () => {
  describe('getWorkflowJsonSchema', () => {
    it('returns a valid JSON schema object', () => {
      const schema = getWorkflowJsonSchema();

      expect(schema).toBeDefined();
      expect(typeof schema).toBe('object');
      expect(schema).toHaveProperty('$schema');
      expect(schema).toHaveProperty('type');
      expect(schema).toHaveProperty('title');
      expect(schema).toHaveProperty('description');
    });

    it('includes standard JSON Schema properties', () => {
      const schema = getWorkflowJsonSchema();

      expect(schema.$schema).toBe('http://json-schema.org/draft-07/schema#');
      expect(schema.title).toBeDefined();
      expect(schema.description).toBeDefined();
    });

    it('returns consistent schema across calls', () => {
      const schema1 = getWorkflowJsonSchema();
      const schema2 = getWorkflowJsonSchema();

      expect(schema1).toEqual(schema2);
    });

    it('expands a schema that refers to itself once, then keeps its reference', () => {
      // A question's `Threshold` may hold a `below` that is another `Threshold`;
      // expanding it without end overflowed the stack.
      const schema = getWorkflowJsonSchema() as Schema;
      const threshold =
        schema.properties!.decision_settings.properties!.monitors.items!.properties!.question
          .properties!.threshold;
      const below = threshold.oneOf!.find((branch) => branch.properties?.below)!;
      expect(below.properties!.below).toEqual({ $ref: '#/components/schemas/Threshold' });
    });
  });

  /**
   * Item 8: a workflow file declares the decision settings and the phase plan its
   * chat starts with. The generated schema the editor documents must describe both
   * blocks as the daemon reads them, strictly: `decision_settings` is the
   * `DecisionSettings` record verbatim (camelCase), `orchestration_plan` the strict
   * `WorkflowPlan` mirror (snake_case), and a key either refuses (`treshold`) is
   * refused here too.
   */
  describe('supervision blocks', () => {
    const supervised = {
      title: 'Echo to thirty',
      description: 'Stops once ten is reached.',
      prompt: 'Using the shell, echo the numbers 1 to 30, one echo command per tool call.',
      decision_settings: {
        defaultModel: {
          provider: 'baseten_decision',
          model: 'decision-models/baseten-joey-2b-latest',
        },
        monitors: [
          {
            id: 'reached-ten',
            name: 'reached-ten',
            question: {
              instructions: 'Has the agent already echoed 10 or a larger number?',
              kind: { binary: {} },
            },
            threshold: 0.7,
            action: 'stop',
            onError: 'pause',
            direction: 'at_or_above',
          },
        ],
      },
      orchestration_plan: {
        title: 'Echo plan',
        goal: 'Echo up to thirty',
        phases: [
          {
            id: 'count',
            title: 'Count',
            kind: 'execution',
            gate: {
              questions: { done: 'Did the agent reach 30?' },
              threshold: 0.9,
              max_attempts: 2,
              model: 'ollama_decision/tev1:0.8b',
              allow_uncalibrated: false,
            },
          },
          { id: 'report', title: 'Report', depends_on: ['count'] },
        ],
      },
    };

    it('a workflow with decision settings and a gated plan validates against the generated schema', () => {
      const schema = getWorkflowJsonSchema() as Schema;
      expect(schema.properties).toHaveProperty('decision_settings');
      expect(schema.properties).toHaveProperty('orchestration_plan');
      expect(violations(schema, supervised)).toEqual([]);
    });

    it('refuses what the daemon refuses: a misspelled gate key, a bad action, a missing question', () => {
      const schema = getWorkflowJsonSchema() as Schema;
      const typo = structuredClone(supervised);
      (typo.orchestration_plan.phases[0].gate as Record<string, unknown>).treshold = 0.9;
      expect(violations(schema, typo)).toEqual([
        '$.orchestration_plan.phases[0].gate.treshold: not allowed',
      ]);
      const action = structuredClone(supervised);
      action.decision_settings.monitors[0].action = 'explode';
      expect(violations(schema, action)).toEqual([
        '$.decision_settings.monitors[0].action: not one of stop, notify, steer',
      ]);
      const noQuestions = structuredClone(supervised);
      delete (noQuestions.orchestration_plan.phases[0].gate as Record<string, unknown>).questions;
      expect(violations(schema, noQuestions)).toEqual([
        '$.orchestration_plan.phases[0].gate: questions is required',
      ]);
      // snake_case is the plan's spelling, camelCase the settings': each refuses the other's.
      const camel = structuredClone(supervised);
      (camel.orchestration_plan.phases[0].gate as Record<string, unknown>).maxAttempts = 2;
      expect(violations(schema, camel)).toEqual([
        '$.orchestration_plan.phases[0].gate.maxAttempts: not allowed',
      ]);
    });
  });
});

/** The part of JSON Schema the generated Workflow schema uses, once its `$ref`s are resolved. */
interface Schema {
  type?: string;
  properties?: Record<string, Schema>;
  required?: string[];
  additionalProperties?: boolean | Schema;
  items?: Schema;
  enum?: unknown[];
  oneOf?: Schema[];
  anyOf?: Schema[];
  nullable?: boolean;
}

/**
 * Every place `value` breaks `schema`, as `$.path: why`. Only what the Workflow
 * schema uses: types, required keys, `additionalProperties`, items, enums,
 * `nullable` and `oneOf`/`anyOf` (one branch must fit).
 */
function violations(schema: Schema, value: unknown, path = '$'): string[] {
  if (value === null || value === undefined) {
    return value === null && !schema.nullable && schema.type ? [`${path}: may not be null`] : [];
  }
  const branches = schema.oneOf ?? schema.anyOf;
  if (branches) {
    const fits = branches.some((branch) => violations(branch, value, path).length === 0);
    return fits ? [] : [`${path}: fits no allowed shape`];
  }
  if (schema.enum && !schema.enum.includes(value)) {
    return [`${path}: not one of ${schema.enum.join(', ')}`];
  }
  switch (schema.type) {
    case 'string':
      return typeof value === 'string' ? [] : [`${path}: not a string`];
    case 'number':
      return typeof value === 'number' ? [] : [`${path}: not a number`];
    case 'integer':
      return Number.isInteger(value) ? [] : [`${path}: not an integer`];
    case 'boolean':
      return typeof value === 'boolean' ? [] : [`${path}: not a boolean`];
    case 'array':
      if (!Array.isArray(value)) return [`${path}: not an array`];
      return schema.items
        ? value.flatMap((item, index) => violations(schema.items!, item, `${path}[${index}]`))
        : [];
    default:
      break;
  }
  if (schema.type === 'object' || schema.properties) {
    if (typeof value !== 'object' || Array.isArray(value)) return [`${path}: not an object`];
    const body = value as Record<string, unknown>;
    const found: string[] = [];
    for (const key of schema.required ?? []) {
      if (!(key in body)) found.push(`${path}: ${key} is required`);
    }
    for (const [key, item] of Object.entries(body)) {
      const property = schema.properties?.[key];
      if (property) found.push(...violations(property, item, `${path}.${key}`));
      else if (schema.additionalProperties === false) found.push(`${path}.${key}: not allowed`);
      else if (typeof schema.additionalProperties === 'object') {
        found.push(...violations(schema.additionalProperties, item, `${path}.${key}`));
      }
    }
    return found;
  }
  return [];
}
