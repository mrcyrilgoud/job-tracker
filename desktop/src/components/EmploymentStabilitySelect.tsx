import { ScoreSelect, type ScoreOption, type ScoreSelectWrapperProps } from "./ScoreSelect";

export const EMPLOYMENT_STABILITY_SCALE_LABEL = "Employment stability (1–5, 5=most secure)";
export const EMPLOYMENT_STABILITY_OPTIONS: readonly ScoreOption[] = [
  { value: "", label: "Unscored", compact: "—" },
  { value: "1", label: "1 — least secure", compact: "1" },
  { value: "2", label: "2", compact: "2" },
  { value: "3", label: "3", compact: "3" },
  { value: "4", label: "4", compact: "4" },
  { value: "5", label: "5 — most secure", compact: "5" },
];

export function EmploymentStabilitySelect(props: ScoreSelectWrapperProps) {
  return (
    <ScoreSelect
      {...props}
      scaleLabel={EMPLOYMENT_STABILITY_SCALE_LABEL}
      options={EMPLOYMENT_STABILITY_OPTIONS}
    />
  );
}
