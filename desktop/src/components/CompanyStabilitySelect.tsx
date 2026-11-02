import { ScoreSelect, type ScoreOption, type ScoreSelectWrapperProps } from "./ScoreSelect";

export const COMPANY_STABILITY_SCALE_LABEL = "Company stability (1–5, 5=most stable)";
export const COMPANY_STABILITY_OPTIONS: readonly ScoreOption[] = [
  { value: "", label: "Unscored", compact: "—" },
  { value: "1", label: "1 — least stable", compact: "1" },
  { value: "2", label: "2", compact: "2" },
  { value: "3", label: "3", compact: "3" },
  { value: "4", label: "4", compact: "4" },
  { value: "5", label: "5 — most stable", compact: "5" },
];
export function CompanyStabilitySelect(props: ScoreSelectWrapperProps) {
  return (
    <ScoreSelect
      {...props}
      scaleLabel={COMPANY_STABILITY_SCALE_LABEL}
      options={COMPANY_STABILITY_OPTIONS}
    />
  );
}
