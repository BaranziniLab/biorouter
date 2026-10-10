import { useState, useEffect } from 'react';
import { transcriptCopy } from './transcript/copy';

const copy = transcriptCopy.image;

type TempPathProps = { kind: 'temp-path'; path: string; alt?: string; className?: string };
type DataProps = { kind: 'data'; data: string; mimeType: string; alt?: string; className?: string };
export type InlineImageProps = TempPathProps | DataProps;

export function InlineImage(props: InlineImageProps) {
  const [isExpanded, setIsExpanded] = useState(false);
  const [error, setError] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [imageData, setImageData] = useState<string | null>(null);

  const alt = props.alt ?? 'Image';
  const className = props.className ?? '';

  // Stable key that changes when the image source changes, regardless of kind.
  const kind = props.kind;
  const sourceKey = kind === 'data' ? props.data : props.path;
  const mimeType = kind === 'data' ? props.mimeType : null;

  useEffect(() => {
    let cancelled = false;

    if (kind === 'data') {
      const dataUrl = `data:${mimeType};base64,${sourceKey}`;
      setImageData(dataUrl);
      setIsLoading(false);
      setError(false);
      return;
    }

    // kind === 'temp-path'
    setImageData(null);
    setIsLoading(true);
    setError(false);

    const loadImage = async () => {
      try {
        const data = await window.electron.getTempImage(sourceKey);
        if (cancelled) return;
        if (data) {
          setImageData(data);
          setIsLoading(false);
        } else {
          setError(true);
          setIsLoading(false);
        }
      } catch (err) {
        console.error('Error loading image:', err);
        if (!cancelled) {
          setError(true);
          setIsLoading(false);
        }
      }
    };

    loadImage();

    return () => {
      cancelled = true;
    };
  }, [kind, sourceKey, mimeType]);

  const handleError = () => {
    setError(true);
    setIsLoading(false);
  };

  const toggleExpand = () => {
    if (!error) {
      setIsExpanded(!isExpanded);
    }
  };

  // For temp-path, validate it is a safe file path
  if (props.kind === 'temp-path' && !props.path.includes('biorouter-pasted-images')) {
    return (
      <div className="my-1 text-supporting text-text-danger">{copy.invalidPath(props.path)}</div>
    );
  }

  if (error) {
    const label = props.kind === 'temp-path' ? props.path : `${props.mimeType} image`;
    return <div className="my-1 text-supporting text-text-danger">{copy.unavailable(label)}</div>;
  }

  return (
    <div className={`image-preview my-2 ${className}`}>
      {isLoading && (
        // A still placeholder the image's size: a load is not work in
        // progress the eye needs to watch, so nothing pulses.
        <div className="flex h-40 w-40 items-center justify-center rounded-element bg-background-medium">
          <span className="text-supporting text-text-muted">{copy.loading}</span>
        </div>
      )}
      {imageData && (
        <button
          type="button"
          onClick={toggleExpand}
          className={`block overflow-hidden rounded-element transition-opacity duration-[var(--dur-fast-min)] hover:opacity-90 ${isExpanded ? 'max-w-full' : 'max-w-40 max-h-40'} ${isLoading ? 'hidden' : ''}`}
          aria-label={isExpanded ? copy.collapse : copy.expand}
        >
          <img
            src={imageData}
            alt={alt}
            onError={handleError}
            className={
              isExpanded ? 'max-h-96 max-w-full object-contain' : 'max-h-40 max-w-40 object-contain'
            }
          />
        </button>
      )}
    </div>
  );
}
