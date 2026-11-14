export enum AlertType {
  Error = 'error',
  Warning = 'warning',
  Info = 'info',
}

export interface Alert {
  type: AlertType;
  /** The one visible line (design.md 9.16): what happened, or the count that matters. */
  message: string;
  /**
   * Helper text behind an info icon beside `message`: why it matters, what it applies to. Plain
   * text, two full sentences at most (about 160 characters), never the only carrier of an error's
   * first line or a refusal. Drawn only on a message alert, not on the progress meter.
   */
  help?: string;
  autoShow?: boolean;
  action?: {
    text: string;
    onClick: () => void;
  };
  progress?: {
    current: number;
    total: number;
  };
  showCompactButton?: boolean;
  compactButtonDisabled?: boolean;
  onCompact?: () => void;
  compactIcon?: React.ReactNode;
  onThresholdChange?: (threshold: number) => void;
}
