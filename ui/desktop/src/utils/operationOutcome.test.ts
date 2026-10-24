import { describe, expect, it } from 'vitest';
import {
  isCodeExecutionTool,
  operationFromToolResult,
  operationFailureText,
  parseOperationOutcome,
  OPERATION_META_KEY,
} from './operationOutcome';

const failed = {
  version: 1,
  id: 'fixture-job',
  status: 'failed',
  error: {
    code: 'fixture_failed',
    kind: 'tool_failure',
    message: 'Synthetic operation failure',
    phase: 'execution',
    retryable: false,
  },
};

describe('operation outcome metadata', () => {
  it('accepts nested telemetry only from the built-in Code Execution identity', () => {
    expect(isCodeExecutionTool('code_execution__execute_code')).toBe(true);
    expect(isCodeExecutionTool('execute_code')).toBe(true);
    expect(isCodeExecutionTool('external__execute_code')).toBe(false);
    expect(isCodeExecutionTool('code_execution__status')).toBe(false);
  });

  it('reads only the versioned result metadata contract', () => {
    const result = {
      status: 'success',
      value: { isError: false, _meta: { [OPERATION_META_KEY]: failed } },
    };
    expect(operationFromToolResult(result)).toEqual(failed);
    expect(result.value.isError).toBe(false);
    expect(operationFromToolResult({ status: 'failed', error: 'ordinary user JSON' })).toBeNull();
    expect(
      operationFromToolResult({
        status: 'success',
        value: { structuredContent: failed, content: [{ text: JSON.stringify(failed) }] },
      })
    ).toBeNull();
    expect(operationFromToolResult({ status: 'error', value: result.value })).toBeNull();
  });

  it.each([
    null,
    'failed',
    { ...failed, error: null },
    { ...failed, error: { ...failed.error, phase: null } },
    { ...failed, version: 2 },
    { ...failed, status: 'unknown' },
    { ...failed, id: ' ' },
    { ...failed, unexpected: true },
    { ...failed, error: 'failure' },
    { ...failed, error: { code: 'x', kind: 'tool_failure', message: 'failure' } },
    { ...failed, error: { ...failed.error, retryable: 'false' } },
  ])('ignores malformed or unknown envelopes: %j', (value) => {
    expect(parseOperationOutcome(value)).toBeNull();
  });

  it.each(['queued', 'running', 'completed', 'cancelled'])(
    'does not turn %s into a failure',
    (status) => {
      expect(operationFailureText(parseOperationOutcome({ ...failed, status }))).toBeNull();
    }
  );

  it('preserves the stable code and original message without guessing from text', () => {
    const parsed = parseOperationOutcome({
      ...failed,
      error: { ...failed.error, kind: 'new_connector_kind', message: '404 records processed' },
    });
    expect(parsed?.error?.kind).toBe('new_connector_kind');
    expect(operationFailureText(parsed)).toBe(
      'Operation fixture-job failed (fixture_failed): 404 records processed'
    );
  });
});
