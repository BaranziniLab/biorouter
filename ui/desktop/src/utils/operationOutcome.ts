export const OPERATION_META_KEY = 'io.biorouter/operation';

export type OperationOutcome = {
  version: 1;
  id: string;
  status: 'queued' | 'running' | 'completed' | 'failed' | 'cancelled';
  error?: { code: string; kind: string; message: string; phase?: string; retryable: boolean };
};

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function nonempty(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0;
}

export function parseOperationOutcome(value: unknown): OperationOutcome | null {
  const operation = record(value);
  if (
    !operation ||
    operation.version !== 1 ||
    !nonempty(operation.id) ||
    typeof operation.status !== 'string' ||
    !['queued', 'running', 'completed', 'failed', 'cancelled'].includes(operation.status) ||
    Object.keys(operation).some((key) => !['version', 'id', 'status', 'error'].includes(key))
  )
    return null;
  let error: OperationOutcome['error'];
  if (operation.error === null) return null;
  if (operation.error !== undefined) {
    const candidate = record(operation.error);
    if (
      !candidate ||
      !nonempty(candidate.code) ||
      !nonempty(candidate.kind) ||
      !nonempty(candidate.message) ||
      typeof candidate.retryable !== 'boolean' ||
      (candidate.phase !== undefined && !nonempty(candidate.phase)) ||
      Object.keys(candidate).some(
        (key) => !['code', 'kind', 'message', 'phase', 'retryable'].includes(key)
      )
    )
      return null;
    error = {
      code: candidate.code,
      kind: candidate.kind,
      message: candidate.message,
      retryable: candidate.retryable,
      ...(candidate.phase != null ? { phase: candidate.phase as string } : {}),
    };
  }
  return {
    version: 1,
    id: operation.id,
    status: operation.status as OperationOutcome['status'],
    ...(error ? { error } : {}),
  };
}

export function operationFromToolResult(result: unknown): OperationOutcome | null {
  const wrapper = record(result);
  if (wrapper?.status !== 'success') return null;
  const value = record(wrapper.value);
  return parseOperationOutcome(record(value?._meta)?.[OPERATION_META_KEY]);
}

export function operationFailureText(operation: OperationOutcome | null): string | null {
  if (operation?.status !== 'failed') return null;
  return operation.error
    ? `Operation ${operation.id} failed (${operation.error.code}): ${operation.error.message}`
    : `Operation ${operation.id} failed without an error message.`;
}

export function isCodeExecutionTool(name: string): boolean {
  return name === 'execute_code' || name === 'code_execution__execute_code';
}
