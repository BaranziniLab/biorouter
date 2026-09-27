import { institutionId, institutionLabel, type KnownInstitution } from './institution';

export interface InstitutionNameProps {
  id: string | null | undefined;
  /** Institutions configured providers publish names for. */
  known?: readonly KnownInstitution[] | null;
  /**
   * Show the exact stored ID in mono — for a confirmation that writes it. With a
   * published name as well, reads `UCSF (ucsf)` with the ID in mono.
   */
  raw?: boolean;
  /** Layout only. */
  className?: string;
}

/**
 * An institution, as `institutionLabel` words it, or — in a confirmation — as
 * the raw ID in mono. Renders nothing when no institution is set; the caller
 * owns the copy for that case.
 */
export function InstitutionName({ id, known, raw = false, className }: InstitutionNameProps) {
  const value = institutionId(id);
  if (value === null) return null;
  const label = institutionLabel(value, known) ?? value;
  const code = (
    <code className="font-mono" data-institution-part="id" translate="no">
      {value}
    </code>
  );

  if (!raw) {
    return (
      <bdi className={className} data-institution-part="label" translate="no">
        {label}
      </bdi>
    );
  }
  if (label === value) return <span className={className}>{code}</span>;
  return (
    <span className={className}>
      <bdi data-institution-part="label">{label}</bdi> ({code})
    </span>
  );
}
