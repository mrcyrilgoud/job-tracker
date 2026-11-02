export type ScoreOption = {
  value: "" | "1" | "2" | "3" | "4" | "5";
  label: string;
  compact: string;
};

export type ScoreSelectProps = {
  id: string;
  value: number | null;
  onChange: (next: number | null) => void;
  compact?: boolean;
  disabled?: boolean;
  /** Used for both `aria-label` and `title`. */
  scaleLabel: string;
  options: readonly ScoreOption[];
};

export type ScoreSelectWrapperProps = Omit<ScoreSelectProps, "scaleLabel" | "options">;

export function ScoreSelect({
  id,
  value,
  onChange,
  compact = false,
  disabled = false,
  scaleLabel,
  options,
}: ScoreSelectProps) {
  return (
    <select
      id={id}
      aria-label={scaleLabel}
      title={scaleLabel}
      value={value ?? ""}
      disabled={disabled}
      onMouseDown={(event) => event.stopPropagation()}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
      }}
      onChange={(event) => {
        event.stopPropagation();
        const raw = event.target.value;
        onChange(raw === "" ? null : Number(raw));
      }}
      className={
        compact
          ? "w-16 rounded-lg border border-[var(--border)] bg-[var(--surface)] px-2 py-1 text-xs text-[var(--foreground)] focus:border-[var(--accent)] focus:outline-none focus:shadow-[0_0_0_3px_color-mix(in_srgb,var(--accent)_18%,transparent)]"
          : "field"
      }
    >
      {options.map((option) => (
        <option key={option.value || "unscored"} value={option.value}>
          {compact ? option.compact : option.label}
        </option>
      ))}
    </select>
  );
}
