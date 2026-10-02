export const APPEAL_SCALE_LABEL = "Appeal (1–5, 5=best)";

const OPTIONS: Array<{ value: string; label: string; compact: string }> = [
  { value: "", label: "Unscored", compact: "—" },
  { value: "1", label: "1 — least appealing", compact: "1" },
  { value: "2", label: "2", compact: "2" },
  { value: "3", label: "3", compact: "3" },
  { value: "4", label: "4", compact: "4" },
  { value: "5", label: "5 — most appealing", compact: "5" },
];

export function AppealSelect({
  id,
  value,
  onChange,
  compact = false,
  disabled = false,
}: {
  id: string;
  value: number | null;
  onChange: (next: number | null) => void;
  compact?: boolean;
  disabled?: boolean;
}) {
  return (
    <select
      id={id}
      aria-label={APPEAL_SCALE_LABEL}
      title={APPEAL_SCALE_LABEL}
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
      {OPTIONS.map((option) => (
        <option key={option.value || "unscored"} value={option.value}>
          {compact ? option.compact : option.label}
        </option>
      ))}
    </select>
  );
}
