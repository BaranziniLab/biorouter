interface OnboardingSectionLabelProps {
  category: 'institutional' | 'local' | 'commercial';
  label: string;
}

const DOT_CLASSES: Record<OnboardingSectionLabelProps['category'], string> = {
  institutional: 'bg-background-info',
  local: 'bg-background-success',
  commercial: 'bg-background-warning',
};

export default function OnboardingSectionLabel({ category, label }: OnboardingSectionLabelProps) {
  return (
    <div className="flex items-center gap-2 mb-1">
      <span className={`w-1.5 h-1.5 rounded-full ${DOT_CLASSES[category]}`} aria-hidden />
      <p className="text-caps text-text-muted">{label}</p>
    </div>
  );
}
