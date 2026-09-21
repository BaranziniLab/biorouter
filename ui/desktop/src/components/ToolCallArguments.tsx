import { ToolContentPreview } from './ToolContentPreview';

export type ToolCallArgumentValue =
  | string
  | number
  | boolean
  | null
  | ToolCallArgumentValue[]
  | { [key: string]: ToolCallArgumentValue };

interface ToolCallArgumentsProps {
  args?: Record<string, ToolCallArgumentValue> | null;
}

export function ToolCallArguments({ args }: ToolCallArgumentsProps) {
  if (!args || typeof args !== 'object' || Array.isArray(args)) return null;
  return (
    <div className="my-2 space-y-2">
      {Object.entries(args).map(([key, value]) => (
        <div key={key} className="flex min-w-0 flex-wrap gap-x-3 gap-y-1">
          <span className="min-w-[100px] shrink-0 text-secondary text-text-muted">{key}</span>
          <div className="min-w-0 flex-1 basis-[200px]">
            <ToolContentPreview
              text={typeof value === 'string' ? value : JSON.stringify(value, null, 2)}
            >
              {(text) => (
                <pre className="min-w-0 max-w-full whitespace-pre-wrap break-words font-mono text-code text-text-muted [overflow-wrap:anywhere]">
                  {text}
                </pre>
              )}
            </ToolContentPreview>
          </div>
        </div>
      ))}
    </div>
  );
}
