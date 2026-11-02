import { ScoreSelect, type ScoreOption, type ScoreSelectWrapperProps } from "./ScoreSelect";

export const APPLICATION_EASE_SCALE_LABEL = "Application ease (1–5, 5=easiest)";
const OPTIONS: readonly ScoreOption[] = [
  { value: "", label: "Unscored", compact: "—" },
  { value: "1", label: "1 — hardest to apply", compact: "1" },
  { value: "2", label: "2", compact: "2" },
  { value: "3", label: "3", compact: "3" },
  { value: "4", label: "4", compact: "4" },
  { value: "5", label: "5 — easiest to apply", compact: "5" },
];
export function ApplicationEaseSelect(props: ScoreSelectWrapperProps) {
  return <ScoreSelect {...props} scaleLabel={APPLICATION_EASE_SCALE_LABEL} options={OPTIONS} />;
}
